//! `template_fork` (docs/mcp.md): a template copied for the person, which
//! every other tool then names. A template is a game's folder holding
//! `template.toml` and a model and nothing from a ROM: each folder under
//! the repo's `templates/` (Moss Hop and the games made here) and each game
//! folder of the games checkout (Donkey Kong, Tapper) is one.
//!
//! The whole folder is copied, subfolders too (`level-kit/`, `baselines/`,
//! `themes/`, `ext/`), leaving out what never belongs in a copy: build
//! output, git's files and any file that is a ROM image. Then the copy's
//! lifted assets (its art, sound and levels) are made when that is cheap
//! and offline: built from `authored.toml` for a game made here, lifted
//! from the person's own ROM when one is found. Otherwise the reply says
//! the next step, so a copy is never left unable to run without a word.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use gp_model::MODEL_FILE;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{Config, Reply};

/// The manifest every template holds.
pub const MANIFEST: &str = "template.toml";

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ForkArgs {
    /// The template: its folder's name under the repo's templates/ or in
    /// the games checkout (`mosshop`, `donkey-kong`), or its template id
    /// (`donkey-kong-oe`).
    pub template: String,
    /// The new copy's name: letters, digits and `_`.
    pub copy: String,
}

/// Every template folder the tools see, by its folder's name: the repo's
/// `templates/` first, then the games checkout's game folders. A name the
/// repo already has is taken from the repo.
pub fn template_dirs(config: &Config) -> Vec<(String, PathBuf)> {
    let roots = [Some(config.repo.join("templates")), config.games.clone()];
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for root in roots.into_iter().flatten() {
        let mut dirs: Vec<(String, PathBuf)> = fs::read_dir(&root)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.join(MANIFEST).is_file())
            .filter_map(|p| Some((p.file_name()?.to_str()?.to_owned(), p)))
            .collect();
        dirs.sort();
        for (name, dir) in dirs {
            if seen.insert(name.clone()) {
                out.push((name, dir));
            }
        }
    }
    out
}

/// A template's manifest as plain TOML, when it has one that parses.
pub fn manifest(dir: &Path) -> Option<toml::Table> {
    fs::read_to_string(dir.join(MANIFEST)).ok()?.parse().ok()
}

fn get<'a>(m: Option<&'a toml::Table>, table: &str, key: &str) -> Option<&'a str> {
    m?.get(table)?.get(key)?.as_str()
}

/// A template by its folder's name or its template id.
pub fn find(config: &Config, name: &str) -> Result<PathBuf, String> {
    if !crate::is_source_name(name) {
        return Err(format!("`{name}` isn't a template name"));
    }
    // A folder under templates/ with a model and no manifest is still
    // copied, as template_fork always did.
    let plain = config.repo.join("templates").join(name);
    let dirs = template_dirs(config);
    let found = dirs
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, d)| d.clone())
        .or_else(|| plain.join(MODEL_FILE).is_file().then_some(plain))
        .or_else(|| {
            dirs.iter()
                .find(|(_, d)| get(manifest(d).as_ref(), "template", "id") == Some(name))
                .map(|(_, d)| d.clone())
        });
    let Some(dir) = found else {
        let games = match &config.games {
            Some(g) => format!(" or in the games checkout at {}", g.display()),
            None => " (the server found no games checkout: set GP_GAMES_DIR and start it again to see the ported games)".into(),
        };
        return Err(format!(
            "there's no template `{name}` under the repo's templates/{games}; registry_search lists the templates"
        ));
    };
    if !dir.join(MODEL_FILE).is_file() {
        return Err(format!("{} has no {MODEL_FILE}", dir.display()));
    }
    Ok(dir)
}

/// Every file of a template folder, by its path inside it, sorted: no
/// `.git`, no build output (`target/`), and refusing symlinks. `gp template
/// extract` and `pack` copy the same files.
pub fn template_files(dir: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) -> Result<(), String> {
        let mut entries: Vec<_> = fs::read_dir(dir)
            .map_err(|e| format!("reading {}: {e}", dir.display()))?
            .filter_map(Result::ok)
            .collect();
        entries.sort_by_key(fs::DirEntry::file_name);
        for e in entries {
            let path = e.path();
            let rel = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            let ft = e
                .file_type()
                .map_err(|err| format!("reading {rel}: {err}"))?;
            if ft.is_symlink() {
                return Err(format!(
                    "{rel} is a symlink; a template holds only plain files"
                ));
            } else if ft.is_dir() {
                if e.file_name() == ".git" || e.file_name() == "target" {
                    continue;
                }
                walk(base, &path, out)?;
            } else if ft.is_file() {
                out.push((rel, path));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out)?;
    Ok(out)
}

/// True for a file that starts as an NES image or a zip (an arcade
/// romset) does. A template never holds one; one left in a game's folder
/// by hand isn't copied.
fn is_rom_image(path: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 4];
    fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut head))
        .is_ok()
        && (gp_evidence::source::looks_like_nes(&head) || head == *b"PK\x03\x04")
}

pub fn fork(config: &Config, a: ForkArgs) -> Result<Reply, String> {
    let to = config.copy_dir(&a.copy)?;
    let from = find(config, &a.template)?;
    if to.exists() {
        return Err(format!("the copy `{}` already exists", a.copy));
    }
    let files = template_files(&from).map_err(|e| {
        format!(
            "{e}, so {} isn't copied; put the file itself there first",
            from.display()
        )
    })?;
    // Copied under another name and renamed into place, so a fork that
    // stopped halfway is never taken for a copy.
    let tmp = config
        .work
        .join(format!(".{}.tmp-{}", a.copy, std::process::id()));
    fs::create_dir_all(&tmp).map_err(|e| format!("creating {}: {e}", tmp.display()))?;
    let mut copied = Vec::new();
    let mut left_out = Vec::new();
    for (rel, path) in files {
        if is_rom_image(&path) {
            left_out.push(json!({ "path": rel, "why": "a ROM image; a template never holds one" }));
            continue;
        }
        let dst = tmp.join(&rel);
        if let Some(p) = dst.parent() {
            fs::create_dir_all(p).map_err(|e| format!("creating {}: {e}", p.display()))?;
        }
        fs::copy(&path, &dst).map_err(|e| format!("copying {rel}: {e}"))?;
        copied.push(rel);
    }
    fs::rename(&tmp, &to).map_err(|e| format!("renaming {} into place: {e}", tmp.display()))?;

    let m = manifest(&to);
    let m = m.as_ref();
    let (_, model) = crate::tools::load_model(&to)?;
    let source = model.model.source.clone();
    let title = get(m, "template", "title")
        .map(str::to_owned)
        .unwrap_or_else(|| model.model.title.clone());
    let tier = get(m, "template", "tier");
    let mut out = json!({
        "copy": a.copy,
        "template": a.template,
        "id": get(m, "template", "id"),
        "title": title,
        "tier": tier,
        "from": from,
        "dir": to,
        "files": copied,
    });
    if !left_out.is_empty() {
        out["left_out"] = json!(left_out);
    }
    let (ready, made, next) = make_assets(config, &from, &to, m, source.as_deref(), &title);
    out["ready"] = json!(ready);
    out["assets"] = made;
    if let Some(n) = next {
        out["next"] = json!(n);
    }
    let notes = notes(m, tier);
    if !notes.is_empty() {
        out["notes"] = json!(notes);
    }
    Ok(Reply::ok(out))
}

/// True when a directory holds a lift: its asset map, `assets.toml`.
fn lifted(dir: &Path) -> bool {
    dir.join(gp_assets::MAP_FILE).is_file()
}

/// The copy's lifted assets: there already, made now when that is cheap
/// and offline, or the next step that makes them. Returns whether the
/// copy is ready, what was found or done, and the next step if any.
fn make_assets(
    config: &Config,
    from: &Path,
    copy: &Path,
    m: Option<&toml::Table>,
    source: Option<&str>,
    title: &str,
) -> (bool, Value, Option<String>) {
    let Some(dir) = config.assets_of(copy, source) else {
        return (
            false,
            json!({ "present": false }),
            Some("the model names no source, so there are no assets to make for it; the tools that run it will say what they miss".into()),
        );
    };
    if lifted(&dir) {
        return (true, json!({ "dir": dir, "present": true }), None);
    }
    let found = json!({ "dir": dir, "present": false });
    if config.assets.is_some() {
        return (
            false,
            found,
            Some(format!(
                "the server was started with --assets {}, which holds no lift; start it with the assets of this game, or without --assets",
                dir.display()
            )),
        );
    }
    let how = get(m, "source", "how");
    // The map as the template has it: a path relative to the template's
    // folder may reach outside it (Moss Hop's is the repo's assets/).
    let map = get(m, "recipe", "asset_map").map(|r| from.join(r));
    let ran = |args: &[&std::ffi::OsStr], what: &str, step: String| {
        let mut v = found.clone();
        match crate::run_gp(config, args) {
            Ok(_) if lifted(&dir) => {
                v["present"] = json!(true);
                v["made"] = json!(what);
                (true, v, None)
            }
            Ok((code, j)) => {
                v["tried"] = json!(what);
                v["error"] = json!(format!("gp exited {code}: {j}"));
                (false, v, Some(step))
            }
            Err(e) => {
                v["tried"] = json!(what);
                v["error"] = json!(e);
                (false, v, Some(step))
            }
        }
    };
    match how {
        Some("authored") => {
            let step = format!(
                "build its assets from its authored.toml: gp template build {}",
                copy.display()
            );
            ran(
                &["template".as_ref(), "build".as_ref(), copy.as_os_str()],
                "built from authored.toml (gp template build)",
                step,
            )
        }
        Some("rom" | "build") => {
            let accepts: Vec<&str> = m
                .and_then(|m| m.get("source")?.get("accepts")?.as_array())
                .into_iter()
                .flatten()
                .filter_map(toml::Value::as_str)
                .collect();
            let (roms, _) = crate::registry::roms_at_hand(config);
            let rom = accepts.iter().find_map(|h| roms.get(*h)).cloned();
            let map_arg = map
                .as_ref()
                .map(|p| format!(" --map {}", p.display()))
                .unwrap_or_default();
            match rom {
                Some(rom) => {
                    let step = format!("lift its assets from the ROM: gp assets lift {rom}{map_arg}");
                    let mut args: Vec<&std::ffi::OsStr> =
                        vec!["assets".as_ref(), "lift".as_ref(), rom.as_ref()];
                    if let Some(p) = &map {
                        args.push("--map".as_ref());
                        args.push(p.as_os_str());
                    }
                    ran(&args, "lifted from the person's own ROM (gp assets lift)", step)
                }
                None if how == Some("build") => (
                    false,
                    found,
                    Some(format!(
                        "build its ROM from source ({}), then lift its assets with gp assets lift ROM{map_arg}, or start the server with --rom and --assets",
                        get(m, "source", "build").unwrap_or("see template.toml"),
                    )),
                ),
                None => (
                    false,
                    found,
                    Some(format!(
                        "this copy needs the person's own ROM of {title}, which isn't in $GP_ROM_DIR: ask them for it, add it with gp roms add FILE, then lift its assets with gp assets lift ROM{map_arg}"
                    )),
                ),
            }
        }
        _ => (
            false,
            found,
            Some("its template.toml doesn't say how its assets are made (source.how), so the tools can't make them; start the server with --assets".into()),
        ),
    }
}

/// What else the copy can't do yet, in plain words.
fn notes(m: Option<&toml::Table>, tier: Option<&str>) -> Vec<String> {
    let mut out = Vec::new();
    if tier == Some("commercial") {
        out.push("This is a commercial game's template: its art, sound and levels come from the person's own ROM and stay on this computer. build_publish refuses the copy while any slot still holds the original's art. assets_generate and assets_place replace art, but they and validate need recorded runs in the repo's baselines/<source>, which only Moss Hop has today.".to_owned());
    }
    let hooks: Vec<&str> = m
        .and_then(|m| m.get("ext")?.get("hooks")?.as_array())
        .into_iter()
        .flatten()
        .filter_map(toml::Value::as_str)
        .collect();
    if !hooks.is_empty() {
        out.push(format!(
            "This game has custom code (Rust hooks in ext/: {}). run_preview, validate and the other tools play the model text alone, where those hooks do nothing; the game plays whole only as its built module (gp game build).",
            hooks.join(", ")
        ));
    }
    if get(m, "source", "platform") == Some("arcade") {
        out.push("An arcade game's model draws nothing yet (no model slot takes an arcade board's pictures), so the copy has no picture to show or replace.".to_owned());
    }
    out
}
