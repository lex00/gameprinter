//! What the interpreter preview loads, written from a model and its lifted
//! assets ([`prep`], gp-web-prep's code), and a visitor's own ROM lifted
//! into it ([`lift_rom`]), for the public page (docs/web.md, "The public
//! page"). Everything goes through `std::fs`, so the same code runs
//! natively and on wasm32-wasip1 over the page's in-memory file system.

use std::path::{Path, PathBuf};

pub mod found;

use gp_interp::lifted::Bundle;
use gp_model::Model;

/// What [`prep`] made, for a caller that goes on with it (gp-web-prep runs
/// its baselines on the bundle).
pub struct Prepped {
    pub model: Model,
    pub bundle: Bundle,
    /// The looks that couldn't be drawn, so the settings don't offer them.
    pub skipped: Vec<Skipped>,
}

/// A theme the settings don't offer, with why: `reason` in words a player
/// reads, written to follow "because" (the public page shows "ROM theme not
/// offered: TITLE, because REASON."), and `detail` the tool's own message,
/// for the theme's maker.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Skipped {
    pub id: String,
    pub title: String,
    pub reason: String,
    pub detail: String,
}

/// The reason for a theme whose files don't load (a missing file, a
/// file's hash that differs from theme.toml's, a value out of range).
pub const NOT_READ: &str = "its files have a mistake in them";

/// The reason for a theme that loads but can't be drawn over the game's
/// art (a picture of the wrong size, a slot the game doesn't have).
fn not_drawn(lifted: bool) -> String {
    format!(
        "it doesn't fit {}",
        if lifted {
            "the art read from your ROM"
        } else {
            "this game's art"
        }
    )
}

/// The themes in `themes_dir` (each folder with a theme.toml, or with a
/// same_as.toml pointing at another template's theme), loaded as looks
/// for the model `model`, and those that don't load with why. A theme
/// whose id is `original` is left out, as `gp_look::looks::offered`
/// leaves it.
pub fn template_looks(
    themes_dir: &Path,
    model: &str,
) -> (Vec<gp_look::looks::LookSpec>, Vec<Skipped>) {
    let mut looks = Vec::new();
    let mut skipped = Vec::new();
    let Ok(rd) = std::fs::read_dir(themes_dir) else {
        return (looks, skipped);
    };
    // A finished look (`ready_look`) goes in as it is, by `add_ready_looks`.
    let mut dirs: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| gp_theme::same_as::is_theme_dir(p) && ready_look(p).is_none())
        .collect();
    dirs.sort();
    for d in dirs {
        match gp_look::looks::LookSpec::theme_for(&d, model) {
            Ok(l) if l.id == gp_present::looks::ORIGINAL => {}
            Ok(l) => looks.push(l),
            Err(e) => {
                // The id and title as theme.toml gives them (the pointed-at
                // one's for a pointer), when it parses.
                let file = gp_theme::same_as::target(&d)
                    .unwrap_or_else(|_| d.clone())
                    .join(gp_theme::format::THEME_FILE);
                let table: Option<toml::Table> = std::fs::read_to_string(file)
                    .ok()
                    .and_then(|t| t.parse().ok());
                let field = |k: &str| {
                    table
                        .as_ref()
                        .and_then(|t| t.get(k)?.as_str().map(str::to_owned))
                };
                let folder = d
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let id = field("id").unwrap_or(folder);
                skipped.push(Skipped {
                    title: field("title").unwrap_or_else(|| id.clone()),
                    id,
                    reason: NOT_READ.into(),
                    detail: e,
                });
            }
        }
    }
    (looks, skipped)
}

/// A finished look in a template's themes folder: a folder holding
/// `look.txt` and no theme.toml (a large-art look, `gp_present::art`, whose
/// art isn't drawn by gp), or a `same_as.toml` pointing at one. The folder
/// whose files are the look.
fn ready_look(dir: &Path) -> Option<PathBuf> {
    let has_look = |d: &Path| {
        d.join(gp_present::looks::LOOK_FILE).is_file()
            && !d.join(gp_theme::format::THEME_FILE).is_file()
    };
    if has_look(dir) {
        return Some(dir.to_owned());
    }
    let text = std::fs::read_to_string(dir.join(gp_theme::same_as::SAME_AS_FILE)).ok()?;
    let t: toml::Table = text.parse().ok()?;
    let target = dir.join(t.get("theme")?.as_str()?);
    has_look(&target).then_some(target)
}

/// Copies each finished look in `themes_dir` ([`ready_look`]) into
/// `out/themes/<id>/` as it is, lists it in `themes/index.json` beside the
/// looks drawn there, and marks `preview.json` as offering looks. A look
/// whose `look.txt` doesn't read is named in `skipped`.
fn add_ready_looks(
    themes_dir: &Path,
    out: &Path,
    skipped: &mut Vec<Skipped>,
) -> Result<(), String> {
    let Ok(rd) = std::fs::read_dir(themes_dir) else {
        return Ok(());
    };
    let mut dirs: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
    dirs.sort();
    let mut added = Vec::new();
    for d in dirs {
        let Some(src) = ready_look(&d) else { continue };
        let id = d
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let look = std::fs::read_to_string(src.join(gp_present::looks::LOOK_FILE))
            .map_err(|e| e.to_string())
            .and_then(|t| gp_present::looks::Look::read(&t));
        let look = match look {
            Ok(l) if gp_present::looks::valid_id(&id) && id != gp_present::looks::ORIGINAL => l,
            Ok(_) => continue,
            Err(e) => {
                skipped.push(Skipped {
                    title: id.clone(),
                    id,
                    reason: NOT_READ.into(),
                    detail: e,
                });
                continue;
            }
        };
        let mut files = Vec::new();
        list(&src, &src, &mut files)?;
        let to = out.join(gp_present::looks::THEMES_DIR).join(&id);
        for f in files {
            let b = std::fs::read(src.join(&f)).map_err(|e| format!("{f}: {e}"))?;
            write(&to.join(&f), &b)?;
        }
        added.push((id, look.title));
    }
    if added.is_empty() {
        return Ok(());
    }
    let index_file = out
        .join(gp_present::looks::THEMES_DIR)
        .join(gp_present::looks::INDEX_FILE);
    let mut index: Vec<(String, String)> = std::fs::read_to_string(&index_file)
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v["themes"].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|t| {
            Some((
                t["id"].as_str()?.to_owned(),
                t["title"].as_str()?.to_owned(),
            ))
        })
        .collect();
    for (id, title) in added {
        if !index.iter().any(|(i, _)| *i == id) {
            index.push((id, title));
        }
    }
    write(
        &index_file,
        gp_present::looks::write_index(&index).as_bytes(),
    )?;
    let cfg_file = out.join("preview.json");
    let mut cfg: serde_json::Value =
        serde_json::from_str(&read(&cfg_file)?).map_err(|e| format!("preview.json: {e}"))?;
    cfg["looks"] = serde_json::Value::Bool(true);
    write(&cfg_file, cfg.to_string().as_bytes())
}

/// Writes `bundle.json`, `profiles.toml`, `profiles/<preset>.json`,
/// `assets/` (the pictures, `present.txt` and `sound.json`) and
/// `preview.json` for the model in `model_file` with the lifted assets in
/// `assets`, drawn with `theme`. `home` is `$GP_HOME`, where a model that
/// binds sound finds its slots' captures and sound banks; without it a
/// bound slot plays nothing.
///
/// `looks` are the other looks the preview offers in its settings, each
/// drawn into `themes/<id>/` beside the bundle with `themes/index.json`
/// listing them (`gp_look::looks`); `lifted` says the assets came
/// from a ROM, so the settings offer the look `original`.
pub fn prep(
    model_file: &Path,
    assets: &Path,
    out: &Path,
    theme: &gp_theme::Resolved,
    home: Option<&Path>,
    looks: &[gp_look::looks::LookSpec],
    lifted: bool,
) -> Result<Prepped, String> {
    let text = read(model_file)?;
    let model = Model::parse(&text).map_err(|e| format!("{}: {e}", model_file.display()))?;
    let model_dir = model_file.parent().unwrap_or(Path::new("."));
    prep_model(model, model_dir, assets, out, theme, home, looks, lifted)
}

/// [`prep`] with the model already read; `model_dir` is the folder its
/// `profiles.toml` is in.
#[allow(clippy::too_many_arguments)]
pub fn prep_model(
    model: Model,
    model_dir: &Path,
    assets: &Path,
    out: &Path,
    theme: &gp_theme::Resolved,
    home: Option<&Path>,
    looks: &[gp_look::looks::LookSpec],
    lifted: bool,
) -> Result<Prepped, String> {
    let bundle: Bundle = gp_look::bundle(&model, assets)?;
    let spec = gp_interp::compile::compile(&model, &bundle.assets)?;
    let art = gp_look::present::art_themed(&model, &spec, assets, theme)?;
    if !theme.is_empty() {
        let r = gp_theme::readability::check(&art.theme.samples, theme);
        eprint!("{}", r.text());
        if r.verdict == gp_theme::readability::Verdict::Refuse {
            return Err("the theme stack makes the game hard to read".into());
        }
    }

    write(&out.join("bundle.json"), &bundle.to_bytes())?;
    // The model in each of its other profiles, with the same assets.
    let profiles = gp_model::profile::Profiles::read_dir(model_dir)?;
    let mut profile_digests = serde_json::Map::new();
    profile_digests.insert(
        gp_model::profile::BASE.into(),
        gp_look::spec_digest(&spec).into(),
    );
    // The page applies any set of switches itself from the file's text
    // (gp_web::profile_bundle); each preset's bundle is written as well.
    if !profiles.is_empty() {
        write(
            &out.join(gp_model::profile::PROFILES_FILE),
            &std::fs::read(model_dir.join(gp_model::profile::PROFILES_FILE))
                .map_err(|e| e.to_string())?,
        )?;
    }
    for name in profiles.names().iter().skip(1) {
        let m = profiles.apply(&model, name)?;
        let b = Bundle::new(&m, bundle.assets.clone());
        let s = gp_interp::compile::compile(&m, &b.assets)?;
        profile_digests.insert(name.clone(), gp_look::spec_digest(&s).into());
        write(
            &out.join("profiles").join(format!("{name}.json")),
            &b.to_bytes(),
        )?;
    }
    write(
        &out.join("assets").join("present.txt"),
        art.present_text().as_bytes(),
    )?;
    for p in art.pictures.iter().chain(art.theme.overlay.iter()) {
        write(&out.join("assets").join(&p.file), &p.png)?;
    }
    // The model's sound, from its slots' captures under $GP_HOME (none
    // without them): the cues and song sets the preview plays live.
    // A model with hooks may keep a game's sound driver as state and give
    // its APU writes (gp_sim::Ext::apu): it gets an empty bundle to play
    // them with when it binds no sound of its own.
    let bound = model.present.as_ref().is_some_and(|p| p.sound.is_some());
    let mut cues = gp_present::sound::Cues::default();
    let hooked = spec.kinds.iter().any(|k| k.ext.is_some());
    let has_sound = bound || hooked;
    if !bound && hooked {
        write(
            &out.join("assets").join("sound.json"),
            gp_look::sound::web_bundle(&gp_look::sound::SoundArt::default()).as_bytes(),
        )?;
    }
    if bound {
        // Without $GP_HOME (in the browser) there are no captures or banks:
        // a model whose sound is its roles and a driver's APU writes (Donkey
        // Kong's) needs neither, and a slot with no capture plays nothing.
        let sets = home
            .map(|h| gp_look::sound::slot_sets(&model, h))
            .unwrap_or_default();
        let banks = home.map(gp_sound::banks::folders).unwrap_or_default();
        let sound = gp_look::sound::art(&model, &spec, &sets, theme, &banks)?;
        for n in &sound.notes {
            eprintln!("gp-web-prep: sound: {n}");
        }
        for (rel, bytes) in &sound.files {
            if rel.starts_with("assets/sound/roles/") {
                write(&out.join(rel), bytes)?;
            }
        }
        cues = sound.cues.clone();
        write(
            &out.join("assets").join("sound.json"),
            gp_look::sound::web_bundle(&sound).as_bytes(),
        )?;
    }
    // The other looks, beside the bundle, for the settings' theme line.
    let own_look = gp_look::looks::own_look((!theme.is_empty()).then_some(theme));
    let mut index = Vec::new();
    let mut skipped = Vec::new();
    if !looks.is_empty() {
        let own_names: Vec<String> = art
            .present
            .pictures
            .iter()
            .map(|p| p.file.clone())
            .collect();
        let mut own = std::collections::BTreeMap::new();
        own.insert("present.txt".to_owned(), art.present_text().into_bytes());
        for p in art.pictures.iter().chain(art.theme.overlay.iter()) {
            own.insert(p.file.clone(), p.png.clone());
        }
        let mut notes = Vec::new();
        for l in looks {
            let drawn = gp_look::looks::draw(&model, &spec, assets, &cues, l, &mut notes).and_then(
                |(names, drawn)| gp_look::looks::folder(&own_names, &own, l, &names, &drawn),
            );
            match drawn {
                Ok(files) => {
                    for (rel, bytes) in files {
                        write(&out.join(rel), &bytes)?;
                    }
                    index.push((l.id.clone(), l.title.clone()));
                }
                Err(e) => {
                    notes.push(format!("look `{}` not offered: {e}", l.id));
                    skipped.push(Skipped {
                        id: l.id.clone(),
                        title: l.title.clone(),
                        reason: not_drawn(lifted),
                        detail: e,
                    });
                }
            }
        }
        for n in notes {
            eprintln!("gp-web-prep: look: {n}");
        }
        write(
            &out.join(gp_present::looks::THEMES_DIR)
                .join(gp_present::looks::INDEX_FILE),
            gp_present::looks::write_index(&index).as_bytes(),
        )?;
    }
    let screen = &art.present.screen;
    let pictures: Vec<&str> = art
        .present
        .pictures
        .iter()
        .map(|p| p.file.as_str())
        .collect();
    let view = model
        .cameras
        .values()
        .next()
        .map(|c| c.size)
        .unwrap_or([256, 240]);
    // Each entity type's role and size, for particles that follow a role.
    let kinds: serde_json::Map<String, serde_json::Value> = model
        .entities
        .iter()
        .map(|(n, e)| {
            (
                n.clone(),
                serde_json::json!({
                    "role": e.core.role,
                    "size": e.core.size.map(|v| v.raw() >> 16),
                }),
            )
        })
        .collect();
    let preview = serde_json::json!({
        "title": model.model.title,
        // The model's name, which the page keeps its settings by
        // (`gp-settings:NAME` in localStorage, docs/settings.md).
        "model": model.model.name,
        "spec_digest": gp_look::spec_digest(&spec),
        // Whether assets/sound.json is written, so the page doesn't ask
        // for one a model without sound hasn't (a 404 is a console error).
        "sound": has_sound,
        "profiles": profiles.names(),
        // The levels by name, in the model's order, for `?level=NAME`.
        "levels": spec.levels.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(),
        // The switches, by group, for an options list.
        "switches": profiles.groups.iter().map(|g| serde_json::json!({
            "group": g.name,
            "title": g.heading(),
            "switches": g.switches.iter().map(|s| serde_json::json!({
                "name": s.name,
                "description": s.description,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "profile_digests": profile_digests,
        "frame_rate": [model.model.frame_rate.num, model.model.frame_rate.den],
        "view": view,
        "visible": screen.visible(),
        "window": screen.window(2),
        "pictures": pictures,
        "kinds": kinds,
        // The look drawn into assets/ (its id and title), whether the
        // assets were lifted from a ROM (the settings then offer
        // `original`), and whether themes/index.json lists other looks.
        "look": [own_look.0, own_look.1],
        "lifted": lifted,
        "looks": !index.is_empty(),
        "theme": {
            "layers": theme.layers.iter().map(|l| &l.theme.id).collect::<Vec<_>>(),
            "overlay": art.theme.overlay.as_ref().map(|p| &p.file),
            "particles": art.theme.emitters.iter().map(|(role, e)| serde_json::json!({
                "motion": format!("{:?}", e.motion).to_lowercase(),
                "color": e.color,
                "count": e.count,
                "seed": e.seed,
                "role": role,
            })).collect::<Vec<_>>(),
        },
    });
    write(&out.join("preview.json"), preview.to_string().as_bytes())?;
    Ok(Prepped {
        model,
        bundle,
        skipped,
    })
}

/// A ROM the template doesn't accept, with what it does accept, in words a
/// visitor reads.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Refused {
    pub error: String,
    pub expected: Vec<Named>,
    pub got: Named,
}

/// A ROM by its headerless SHA-256, with its registry entry when it has one.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Named {
    pub sha256: String,
    pub id: Option<String>,
    pub title: Option<String>,
    /// The title with its revision, such as `Donkey Kong (JU)`.
    pub name: Option<String>,
}

/// What [`lift_rom`] did.
pub enum Outcome {
    /// Lifted and prepared: the ROM's hash and the files written under OUT,
    /// for a game folder ([`lift_game`]) what was found and loaded, and the
    /// template's themes the settings don't offer, with why.
    Lifted {
        sha256: String,
        files: Vec<String>,
        found: Option<serde_json::Value>,
        looks_skipped: Vec<Skipped>,
    },
    Refused(Refused),
}

#[derive(serde::Deserialize)]
struct Registry {
    #[serde(default)]
    source: Vec<Entry>,
}

#[derive(serde::Deserialize)]
struct Entry {
    id: String,
    title: Option<String>,
    revision: Option<String>,
    sha256: Option<String>,
}

fn named(registry: &[Entry], sha256: &str) -> Named {
    let e = registry.iter().find(|e| {
        e.sha256
            .as_deref()
            .is_some_and(|h| h.eq_ignore_ascii_case(sha256))
    });
    Named {
        sha256: sha256.to_owned(),
        id: e.map(|e| e.id.clone()),
        title: e.and_then(|e| e.title.clone()),
        name: e.and_then(|e| {
            let t = e.title.as_deref()?;
            Some(match &e.revision {
                Some(r) => format!("{t} ({r})"),
                None => t.to_owned(),
            })
        }),
    }
}

/// The template files [`lift_rom`] reads from its directory.
pub const TEMPLATE_FILES: &[&str] = &[
    "model.toml",
    "profiles.toml",
    "template.toml",
    "asset-map.toml",
    "sources-nes.toml",
];

/// Checks `rom` against the template in `template` (its `template.toml`'s
/// `source.accepts`, named through `sources-nes.toml`), lifts it with the
/// template's asset map (`asset-map.toml`, else the manifest's
/// `recipe.asset_map`) into `out/lifted/`, and writes what the preview
/// loads into `out` ([`prep`]).
pub fn lift_rom(rom: &[u8], template: &Path, out: &Path) -> Result<Outcome, String> {
    if template.join(GAME_FILE).is_file() {
        return lift_game(rom, template, out);
    }
    let manifest: toml::Table = toml::from_str(&read(&template.join("template.toml"))?)
        .map_err(|e| format!("template.toml: {e}"))?;
    let accepts: Vec<String> = manifest
        .get("source")
        .and_then(|s| s.get("accepts"))
        .and_then(|a| a.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let registry: Vec<Entry> = match std::fs::read_to_string(template.join("sources-nes.toml")) {
        Ok(t) => {
            toml::from_str::<Registry>(&t)
                .map_err(|e| format!("sources-nes.toml: {e}"))?
                .source
        }
        Err(_) => Vec::new(),
    };
    let expected: Vec<Named> = accepts.iter().map(|h| named(&registry, h)).collect();
    let want = expected
        .iter()
        .map(|n| match &n.name {
            Some(name) => format!("{name}, sha256 {}", n.sha256),
            None => format!("sha256 {}", n.sha256),
        })
        .collect::<Vec<_>>()
        .join(" or ");
    let label = expected
        .first()
        .and_then(|n| n.name.clone())
        .unwrap_or_else(|| "accepted".into());
    let image = match gp_evidence::source::identify_nes(rom) {
        Ok(i) => i,
        Err(e) => {
            return Ok(Outcome::Refused(Refused {
                error: format!(
                    "This file isn't an NES ROM ({e}). This page accepts the {label} ROM: {want}."
                ),
                expected,
                got: Named::default(),
            }));
        }
    };
    if !accepts
        .iter()
        .any(|h| h.eq_ignore_ascii_case(&image.sha256))
    {
        let got = named(&registry, &image.sha256);
        let what = match &got.name {
            Some(n) => format!("This is {n}, not the {label} ROM this page accepts"),
            None => format!("This is not the {label} ROM this page accepts"),
        };
        return Ok(Outcome::Refused(Refused {
            error: format!("{what}. Expected {want}; this file's is {}.", image.sha256),
            expected,
            got,
        }));
    }

    let map_file = match template.join("asset-map.toml") {
        p if p.is_file() => p,
        _ => {
            let rel = manifest
                .get("recipe")
                .and_then(|r| r.get("asset_map"))
                .and_then(|v| v.as_str())
                .ok_or("template.toml has no recipe.asset_map, and there's no asset-map.toml")?;
            template.join(rel)
        }
    };
    let map_text = read(&map_file)?;
    let map = gp_assets::AssetMap::parse(&map_text)
        .map_err(|e| format!("{}: {e}", map_file.display()))?;
    let (files, _) = gp_assets::lift(rom, &map, &map_text).map_err(|e| e.to_string())?;
    let lifted = out.join("lifted");
    for f in &files {
        write(&lifted.join(&f.name), &f.bytes)?;
    }
    // The template's themes, each drawn on the visitor's lifted art as a
    // look the settings offer beside the original.
    let model_file = template.join(gp_model::MODEL_FILE);
    let model_name = Model::parse(&read(&model_file)?)
        .map_err(|e| format!("{}: {e}", model_file.display()))?
        .model
        .name;
    let (looks, mut looks_skipped) =
        template_looks(&template.join(gp_present::looks::THEMES_DIR), &model_name);
    let prepped = prep(
        &template.join(gp_model::MODEL_FILE),
        &lifted,
        out,
        &gp_theme::Resolved::none(),
        None,
        &looks,
        true,
    )?;
    looks_skipped.extend(prepped.skipped);
    add_ready_looks(
        &template.join(gp_present::looks::THEMES_DIR),
        out,
        &mut looks_skipped,
    )?;
    let mut written = Vec::new();
    list(out, out, &mut written)?;
    written.retain(|f| !f.starts_with("lifted/") && f != "result.json");
    written.sort();
    Ok(Outcome::Lifted {
        sha256: image.sha256,
        files: written,
        found: None,
        looks_skipped,
    })
}

/// The file that makes a folder a game's: `{ "game", "title",
/// "templates": [id...], "fallback": folder or null }`, each template's
/// files ([`TEMPLATE_FILES`] less the registry, and its themes) in a folder
/// named by its id, `sources-nes.toml` beside them, and the fallback folder
/// holding the paper version's model.toml and lifted files.
pub const GAME_FILE: &str = "game.json";

#[derive(serde::Deserialize)]
struct GameFile {
    game: String,
    title: String,
    templates: Vec<String>,
    #[serde(default)]
    fallback: Option<String>,
    /// The levels the page plays, when it plays only some: each stage of
    /// the model's stages list whose level isn't one of them is played by
    /// the list's stand-in ([`play_only`]).
    #[serde(default)]
    levels: Option<Vec<String>>,
}

/// Each stage of the model's stages list (core `stages`) whose level isn't
/// in `levels` is played by the list's stand-in, as a stage the model has
/// no level for is: the stage counter, the loops and what they set go on
/// as the game's. The levels stay in the model (`?level=NAME` still plays
/// one); only the stages list changes. A model without a stages list, or
/// whose stand-in isn't in `levels`, is left as it is.
pub fn play_only(model: &mut Model, levels: &[String]) {
    let Some(st) = model.stages.as_mut() else {
        return;
    };
    if !levels.contains(&st.stand_in) {
        return;
    }
    for name in &mut st.order {
        if model.levels.contains_key(name.as_str()) && !levels.contains(name) {
            *name = st.stand_in.clone();
        }
    }
}

/// Each stage of the model's stages list whose level the model has but a
/// ROM loaded only to play hasn't (not in `levels`) is one the game passes
/// over (core `stages.skip`): that ROM's own game has no such stage, so
/// the stage counter steps past it. Donkey Kong (JU) has no 50m and goes
/// from 25m to 75m ($53 from 1 to 3), so with a JU ROM Original
/// Edition's model plays 25m, 75m and 100m. The levels stay in the model.
pub fn skip_missing(model: &mut Model, levels: &[String]) {
    let Some(st) = model.stages.as_mut() else {
        return;
    };
    for name in &st.order {
        if model.levels.contains_key(name.as_str())
            && !levels.contains(name)
            && !st.skip.contains(name)
        {
            st.skip.push(name.clone());
        }
    }
}

/// The file of a game folder's template folder that makes it a ROM a
/// template loads only to play (its `[[loads]]`, docs/templates.md "ROMs
/// to play"): `{ "plays", "levels", "assets" }`.
pub const LOAD_FILE: &str = "load.json";

/// A [`LOAD_FILE`].
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Load {
    /// The template whose model the ROM plays.
    pub plays: String,
    /// The model's levels the ROM has; every other stage is one the game
    /// passes over ([`skip_missing`]).
    #[serde(default)]
    pub levels: Vec<String>,
    /// For an asset of the model's the ROM's map lacks, the map's asset
    /// the model takes instead.
    #[serde(default)]
    pub assets: std::collections::BTreeMap<String, String>,
}

impl Load {
    /// The model's text with every slot filled from `source` (the loaded
    /// ROM's), and each that takes an asset in [`Load::assets`] taking the
    /// one it names.
    pub fn model_text(&self, text: &str, source: &str) -> Result<String, String> {
        let mut t: toml::Table = toml::from_str(text).map_err(|e| format!("model.toml: {e}"))?;
        if let Some(slots) = t.get_mut("assets").and_then(|a| a.as_table_mut()) {
            for (_, slot) in slots.iter_mut() {
                let Some(from) = slot.get_mut("from").and_then(|f| f.as_table_mut()) else {
                    continue;
                };
                if from.contains_key("source") {
                    from.insert("source".into(), toml::Value::String(source.to_owned()));
                }
                let Some(asset) = from.get_mut("asset") else {
                    continue;
                };
                if let Some(to) = asset.as_str().and_then(|a| self.assets.get(a)) {
                    *asset = toml::Value::String(to.clone());
                }
            }
        }
        toml::to_string(&t).map_err(|e| e.to_string())
    }
}

/// The model's text with its screens' text (`present.screens.text`) in
/// the font `map` gives the HUD's slot: a ROM loaded only to play has its
/// own font, whose charset may not name a character the model's template
/// writes (Original Edition's turn screen numerals, `|` and `#`, on a
/// Donkey Kong (JU) ROM). Such a character is written as a blank, as a
/// HUD field leaves out one its font hasn't. A font the map doesn't hold
/// (filled from the paper version) or with no charset changes nothing.
fn screen_text_in_font(text: &str, map: &gp_assets::AssetMap) -> Result<String, String> {
    let mut t: toml::Table = toml::from_str(text).map_err(|e| format!("model.toml: {e}"))?;
    let font = t
        .get("present")
        .and_then(|p| p.get("hud"))
        .and_then(|h| h.get("font"))
        .and_then(|f| f.as_str())
        .and_then(|slot| {
            t.get("assets")?
                .get(slot)?
                .get("from")?
                .get("asset")?
                .as_str()
        })
        .and_then(|a| map.asset(a))
        .and_then(|a| a.charset());
    let Some(charset) = font else {
        return Ok(text.to_owned());
    };
    let mut changed = false;
    let screens = t
        .get_mut("present")
        .and_then(|p| p.get_mut("screens"))
        .and_then(|s| s.as_array_mut());
    for sc in screens.into_iter().flatten() {
        let lines = sc.get_mut("text").and_then(|x| x.as_array_mut());
        for line in lines.into_iter().flatten() {
            let Some(toml::Value::String(s)) = line.get_mut("text") else {
                continue;
            };
            if s.chars().all(|c| charset.tile(c).is_some()) {
                continue;
            }
            *s = s
                .chars()
                .map(|c| if charset.tile(c).is_some() { c } else { ' ' })
                .collect();
            changed = true;
        }
    }
    if !changed {
        return Ok(text.to_owned());
    }
    toml::to_string(&t).map_err(|e| e.to_string())
}

/// The template folder's [`LOAD_FILE`], if it has one.
fn read_load(dir: &Path) -> Result<Option<Load>, String> {
    let p = dir.join(LOAD_FILE);
    if !p.is_file() {
        return Ok(None);
    }
    serde_json::from_str(&read(&p)?)
        .map(Some)
        .map_err(|e| format!("{}: {e}", p.display()))
}

/// Any dump of the game in `dir` ([`GAME_FILE`]): the template its program
/// matches ([`found::pick`]), the assets found in it, and the rest filled
/// from the paper version. A ROM no template's program matches is refused.
pub fn lift_game(rom: &[u8], dir: &Path, out: &Path) -> Result<Outcome, String> {
    let game: GameFile = serde_json::from_str(&read(&dir.join(GAME_FILE))?)
        .map_err(|e| format!("{GAME_FILE}: {e}"))?;
    let registry: Vec<Entry> = match std::fs::read_to_string(dir.join("sources-nes.toml")) {
        Ok(t) => {
            toml::from_str::<Registry>(&t)
                .map_err(|e| format!("sources-nes.toml: {e}"))?
                .source
        }
        Err(_) => Vec::new(),
    };
    let mut templates = Vec::new();
    for id in &game.templates {
        let tdir = dir.join(id);
        let map_text = read(&tdir.join("asset-map.toml"))?;
        let map = gp_assets::AssetMap::parse(&map_text)
            .map_err(|e| format!("{id}: asset-map.toml: {e}"))?;
        templates.push(found::Template {
            id: id.clone(),
            dir: tdir,
            map_text,
            map,
        });
    }
    let expected: Vec<Named> = templates
        .iter()
        .map(|t| named(&registry, &t.map.sha256))
        .collect();
    let image = match gp_evidence::source::identify_nes(rom) {
        Ok(i) => i,
        Err(e) => {
            return Ok(Outcome::Refused(Refused {
                error: format!(
                    "This file isn't an NES ROM ({e}). This page takes any {} ROM.",
                    game.title
                ),
                expected,
                got: Named::default(),
            }));
        }
    };
    let picked = found::pick(rom, &templates)?;
    if picked.matched == gp_assets::find::Match::None {
        let got = named(&registry, &image.sha256);
        let what = match &got.name {
            Some(n) => format!("This is {n}, not a {} ROM", game.title),
            None => format!("This doesn't look like a {} ROM", game.title),
        };
        return Ok(Outcome::Refused(Refused {
            error: format!(
                "{what}: at most {:.0}% of the program of a {} version this page knows is in it.",
                picked.share * 100.0,
                game.title
            ),
            expected,
            got,
        }));
    }
    let t = picked.template;
    let lifted = out.join("lifted");
    let model_file = t.dir.join(gp_model::MODEL_FILE);
    // A ROM a template loads only to play ([[loads]]): its model with the
    // assets the ROM's map lacks taken from the ones the load names.
    let load = read_load(&t.dir)?;
    let model_text = match &load {
        Some(l) => screen_text_in_font(&l.model_text(&read(&model_file)?, &t.map.source)?, &t.map)?,
        None => read(&model_file)?,
    };
    // The maps a blank asset's shape is read from (found::fill): the
    // template's own, and the map of the template a loaded ROM plays.
    let plays_map = match &load {
        Some(l) => templates
            .iter()
            .find(|x| x.id == l.plays)
            .map(|x| x.map_text.clone()),
        None => None,
    };
    let shapes: Vec<&str> = std::iter::once(t.map_text.as_str())
        .chain(plays_map.as_deref())
        .collect();
    let mut loads = Vec::new();
    if picked.report.is_source {
        let (files, _) = gp_assets::lift(rom, &t.map, &t.map_text).map_err(|e| e.to_string())?;
        for f in &files {
            write(&lifted.join(&f.name), &f.bytes)?;
        }
        for a in &t.map.assets {
            loads.push(found::AssetLoad {
                name: a.name().to_owned(),
                from: "same",
                with: None,
            });
        }
        // The model's assets the map hasn't (Original Edition's 50m on a
        // Donkey Kong (JU) ROM), from the paper version.
        let names: Vec<String> = t.map.assets.iter().map(|a| a.name().to_owned()).collect();
        if load.is_some() {
            let fb = game
                .fallback
                .as_ref()
                .map(|f| dir.join(f))
                .filter(|f| f.is_dir())
                .ok_or("the model has assets this ROM hasn't, and there is no paper version to fill them from")?;
            for (n, from) in found::fill(
                &model_text,
                &t.map_text,
                &t.map,
                &names,
                &fb,
                &lifted,
                &shapes,
            )? {
                loads.push(found::AssetLoad {
                    name: n,
                    from,
                    with: None,
                });
            }
        }
    } else {
        let names = found::lift_found(rom, &t.map, &t.map_text, &picked.report, &lifted)?;
        let mut filled = Vec::new();
        if names.len() < t.map.assets.len() {
            let fb = game
                .fallback
                .as_ref()
                .map(|f| dir.join(f))
                .filter(|f| f.is_dir())
                .ok_or("some assets weren't found in this ROM, and there is no paper version to fill them from")?;
            filled = found::fill(
                &model_text,
                &t.map_text,
                &t.map,
                &names,
                &fb,
                &lifted,
                &shapes,
            )?;
        }
        let state = |n: &str| {
            picked
                .report
                .assets
                .iter()
                .find(|a| a.name == n)
                .map_or("same", |a| a.state)
        };
        for a in &t.map.assets {
            let n = a.name();
            if names.iter().any(|x| x == n) {
                loads.push(found::AssetLoad {
                    name: n.to_owned(),
                    from: if state(n) == "changed" {
                        "changed"
                    } else {
                        "same"
                    },
                    with: None,
                });
            } else if let Some((_, from)) = filled.iter().find(|(x, _)| x == n) {
                loads.push(found::AssetLoad {
                    name: n.to_owned(),
                    from,
                    with: None,
                });
            }
        }
        // The model's assets the map hasn't (a ROM loaded only to play).
        for (n, from) in &filled {
            if !t.map.assets.iter().any(|a| a.name() == n) {
                loads.push(found::AssetLoad {
                    name: n.clone(),
                    from,
                    with: None,
                });
            }
        }
    }
    let mut model =
        Model::parse(&model_text).map_err(|e| format!("{}: {e}", model_file.display()))?;
    let (looks, mut looks_skipped) = template_looks(
        &t.dir.join(gp_present::looks::THEMES_DIR),
        &model.model.name,
    );
    if let Some(l) = load.as_ref().filter(|l| !l.levels.is_empty()) {
        skip_missing(&mut model, &l.levels);
    }
    if let Some(levels) = &game.levels {
        play_only(&mut model, levels);
    }
    let prepped = prep_model(
        model,
        &t.dir,
        &lifted,
        out,
        &gp_theme::Resolved::none(),
        None,
        &looks,
        true,
    )?;
    looks_skipped.extend(prepped.skipped);
    add_ready_looks(
        &t.dir.join(gp_present::looks::THEMES_DIR),
        out,
        &mut looks_skipped,
    )?;
    let mut written = Vec::new();
    list(out, out, &mut written)?;
    written.retain(|f| !f.starts_with("lifted/") && f != "result.json");
    written.sort();
    let got = named(&registry, &image.sha256);
    let tname = named(&registry, &t.map.sha256);
    let warning = match picked.matched {
        gp_assets::find::Match::Close => Some(format!(
            "This ROM's program is {:.0}% the same as {}'s, changed in places, so the game may not match this ROM's behaviour.",
            picked.share * 100.0,
            tname.name.as_deref().unwrap_or(&t.id)
        )),
        _ => None,
    };
    let info = serde_json::json!({
        "game": game.game,
        "template": t.id,
        // For a ROM loaded only to play: the template whose model it plays.
        "plays": load.as_ref().map(|l| l.plays.clone()),
        "template_name": tname.name,
        "rom_name": got.name,
        "match": picked.matched,
        "program_share": picked.share,
        "warning": warning,
        "assets": loads,
        "compared": picked.reports.iter().map(|(id, r)| serde_json::json!({
            "template": id,
            "program_share": r.code.as_ref().map_or(0.0, |c| c.share()),
            "same": r.count("same"),
            "changed": r.count("changed"),
            "missing": r.count("missing"),
        })).collect::<Vec<_>>(),
    });
    Ok(Outcome::Lifted {
        sha256: image.sha256,
        files: written,
        found: Some(info),
        looks_skipped,
    })
}

/// What `gp-web-lift` writes to `OUT/result.json` for `result`, and its
/// exit code: 0 lifted, 3 the ROM isn't one the template accepts, 2
/// couldn't run. A lifted result's `looks_skipped` lists each theme the
/// settings don't offer, `{ id, title, reason, detail }` ([`Skipped`]).
pub fn result_json(result: Result<Outcome, String>) -> (serde_json::Value, i32) {
    match result {
        Ok(Outcome::Lifted {
            sha256,
            files,
            found,
            looks_skipped,
        }) => (
            serde_json::json!({
                "ok": true,
                "sha256": sha256,
                "files": files,
                "found": found,
                "looks_skipped": looks_skipped,
            }),
            0,
        ),
        Ok(Outcome::Refused(r)) => {
            let mut v = serde_json::to_value(&r).unwrap_or_default();
            v["ok"] = false.into();
            (v, 3)
        }
        Err(e) => (serde_json::json!({ "ok": false, "error": e }), 2),
    }
}

fn list(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for e in entries {
        let e = e.map_err(|e| e.to_string())?;
        let p: PathBuf = e.path();
        if p.is_dir() {
            list(root, &p, out)?;
        } else if let Ok(rel) = p.strip_prefix(root) {
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
    Ok(())
}

fn read(p: &Path) -> Result<String, String> {
    std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))
}

/// Writes a file, making its folder.
pub fn write(p: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    std::fs::write(p, bytes).map_err(|e| format!("{}: {e}", p.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A made-up NROM image: 16KB of PRG and 8KB of CHR counting up.
    fn made_up_rom() -> Vec<u8> {
        let mut rom = b"NES\x1a\x01\x01".to_vec();
        rom.resize(16, 0);
        rom.extend((0..16384 + 8192).map(|i| (i * 7) as u8));
        rom
    }

    #[test]
    fn screen_text_a_loaded_roms_font_has_no_tile_for_is_written_blank() {
        let map = gp_assets::AssetMap::parse(
            "schema = 1\nsource = \"rom\"\nsha256 = \"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"\n[[asset]]\nname = \"tiles\"\nformat = \"chr-2bpp\"\nregion = \"chr\"\noffset = 0\ntiles = 256\ncharset = [[0x00, \"AB \"]]\n",
        )
        .unwrap();
        let model = "[assets.font.from]\nasset = \"tiles\"\n[present.hud]\nfont = \"font\"\n[[present.screens]]\ntext = [{ at = [0, 0], text = \"AB|B\" }, { at = [0, 1], text = \"BA\" }]\n";
        let out = screen_text_in_font(model, &map).unwrap();
        let t: toml::Table = toml::from_str(&out).unwrap();
        let text = &t["present"]["screens"][0]["text"];
        assert_eq!(text[0]["text"].as_str(), Some("AB B"));
        assert_eq!(text[1]["text"].as_str(), Some("BA"));
        // Every character in the font: the text is unchanged.
        let fine = model.replace('|', "A");
        assert_eq!(screen_text_in_font(&fine, &map).unwrap(), fine);
    }

    #[test]
    fn a_rom_the_template_does_not_accept_is_refused_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let t = dir.path();
        std::fs::write(t.join("template.toml"), "[source]\naccepts = [\"00ff\"]\n").unwrap();
        std::fs::write(
            t.join("sources-nes.toml"),
            "[[source]]\nid = \"made-up-j\"\ntitle = \"Made Up\"\nrevision = \"J\"\nsha256 = \"00ff\"\n",
        )
        .unwrap();
        let out = t.join("out");
        let Outcome::Refused(r) = lift_rom(&made_up_rom(), t, &out).unwrap() else {
            panic!("a ROM with another hash was lifted");
        };
        assert!(
            r.error
                .contains("not the Made Up (J) ROM this page accepts"),
            "{}",
            r.error
        );
        assert!(r.error.contains("sha256 00ff"), "{}", r.error);
        assert_eq!(r.expected[0].id.as_deref(), Some("made-up-j"));
        assert!(r.got.id.is_none());
        assert!(!out.exists(), "nothing is written for a refused ROM");

        let Outcome::Refused(r) = lift_rom(b"PK\x03\x04", t, &out).unwrap() else {
            panic!("a zip was lifted");
        };
        assert!(r.error.contains("isn't an NES ROM"), "{}", r.error);
    }

    /// A copy of Rivet Rodeo's template (an original, open template) whose
    /// source is the data image `gp template build` makes from its
    /// authored.toml, so the lift runs on no game's ROM. Its themes are
    /// `paper` (its own) and two that can't be offered: `unread`, whose
    /// restyle's hash isn't its file's, and `small`, whose tiles are 8 by 8
    /// pixels where the game's are 128 by 128.
    fn rivet_rodeo_with_broken_themes(t: &Path) -> Vec<u8> {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let src = repo.join("../games/donkey-kong/open");
        let built = gp_assets::authored::Authored::load(&src)
            .unwrap()
            .build()
            .unwrap();
        for f in ["model.toml", "profiles.toml"] {
            std::fs::copy(src.join(f), t.join(f)).unwrap();
        }
        std::fs::write(
            t.join("template.toml"),
            format!("[source]\naccepts = [\"{}\"]\n", built.sha256),
        )
        .unwrap();
        std::fs::write(t.join("asset-map.toml"), &built.map_text).unwrap();
        let themes = t.join("themes");
        let paper = src.join("themes/paper");
        std::fs::create_dir_all(themes.join("paper")).unwrap();
        for e in std::fs::read_dir(&paper).unwrap() {
            let p = e.unwrap().path();
            std::fs::copy(&p, themes.join("paper").join(p.file_name().unwrap())).unwrap();
        }
        let pix =
            "pix 1\nsize 8 8\n. transparent\n1 #555555\n".to_owned() + &"1.1.1.1.\n".repeat(8);
        let theme = |id: &str, title: &str, sha: &str| {
            format!(
                "format = \"0.1.3\"\nid = \"{id}\"\ntitle = \"{title}\"\nlicense = \"CC0-1.0\"\n\
                 for = [\"rivet_rodeo\"]\n\n[[restyle]]\nslot = \"tiles\"\nfile = \"tiles.pix\"\n\
                 origin = \"drawn\"\nlicense = \"CC0-1.0\"\nsha256 = \"{sha}\"\n"
            )
        };
        for (id, title, sha) in [
            (
                "small",
                "Small Tiles",
                gp_theme::format::sha256_hex(pix.as_bytes()),
            ),
            ("unread", "Unread", "00".repeat(32)),
        ] {
            std::fs::create_dir_all(themes.join(id)).unwrap();
            std::fs::write(themes.join(id).join("tiles.pix"), &pix).unwrap();
            std::fs::write(
                themes.join(id).join(gp_theme::format::THEME_FILE),
                theme(id, title, &sha),
            )
            .unwrap();
        }
        built.image
    }

    /// A theme that doesn't load and one that doesn't draw are each named
    /// in result.json with a reason a player reads and the tool's message;
    /// the lift still succeeds, and offers the theme that draws.
    #[test]
    #[ignore = "needs the games repo beside this one (phase 1.3)"]
    fn themes_the_lift_cannot_offer_are_named_with_why() {
        let dir = tempfile::tempdir().unwrap();
        let t = dir.path().join("template");
        std::fs::create_dir_all(&t).unwrap();
        let rom = rivet_rodeo_with_broken_themes(&t);
        let out = dir.path().join("out");
        let (json, code) = result_json(lift_rom(&rom, &t, &out));
        assert_eq!(code, 0, "{json}");
        let skipped = json["looks_skipped"].as_array().unwrap();
        assert_eq!(skipped.len(), 2, "{json}");
        let by = |id: &str| skipped.iter().find(|s| s["id"] == id).unwrap();
        let small = by("small");
        assert_eq!(small["title"], "Small Tiles");
        assert_eq!(small["reason"], "it doesn't fit the art read from your ROM");
        assert!(
            small["detail"]
                .as_str()
                .unwrap()
                .contains("is 8 by 8, not 128 by 128"),
            "{small}"
        );
        let unread = by("unread");
        assert_eq!(unread["title"], "Unread");
        assert_eq!(unread["reason"], NOT_READ);
        assert!(
            unread["detail"].as_str().unwrap().contains("SHA-256"),
            "{unread}"
        );
        // The theme that draws is offered, and the two that don't aren't.
        let index = read(&out.join("themes/index.json")).unwrap();
        assert!(index.contains("\"paper\""), "{index}");
        assert!(
            !index.contains("small") && !index.contains("unread"),
            "{index}"
        );
    }

    /// Two made-up versions of Rivet Rodeo's template side by side, `a`
    /// (model rivet_rodeo) and `b` (the same, named rivet_rodeo_b), whose
    /// paper theme is kept once, in `a`, with `for` naming both; `b`'s
    /// themes/paper/ holds only same_as.toml. Each is lifted from its data
    /// image and offers the paper theme, drawn the same. (`a` keeps the
    /// two themes it can't offer; they are skipped there.)
    #[test]
    #[ignore = "needs the games repo beside this one (phase 1.3)"]
    fn a_finished_look_goes_in_as_it_is_on_both_models() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("templates");
        let (a, b) = (root.join("a"), root.join("b"));
        std::fs::create_dir_all(&a).unwrap();
        let rom = rivet_rodeo_with_broken_themes(&a);
        // A large-art look: look.txt naming its rules, its art, no theme.toml.
        let clay = a.join("themes/clay");
        write(
            &clay.join("look.txt"),
            b"gp-look 1\nid clay\ntitle Clay\nart art.txt\n",
        )
        .unwrap();
        write(&clay.join("art.txt"), b"gp-art 1\nscale 6\n").unwrap();
        write(&clay.join("art/hero.png"), b"not drawn by gp").unwrap();
        // Another template points at it.
        std::fs::create_dir_all(b.join("themes/clay")).unwrap();
        for f in [
            "template.toml",
            "asset-map.toml",
            "profiles.toml",
            "model.toml",
        ] {
            std::fs::copy(a.join(f), b.join(f)).unwrap();
        }
        std::fs::write(
            b.join("themes/clay").join(gp_theme::same_as::SAME_AS_FILE),
            "theme = \"../../../a/themes/clay\"\n",
        )
        .unwrap();
        for t in [&a, &b] {
            let out = dir.path().join("out").join(t.file_name().unwrap());
            let (json, code) = result_json(lift_rom(&rom, t, &out));
            assert_eq!(code, 0, "{json}");
            let skipped = json["looks_skipped"].as_array().unwrap();
            assert!(!skipped.iter().any(|s| s["id"] == "clay"), "{json}");
            let index = read(&out.join("themes/index.json")).unwrap();
            assert!(
                index.contains("{\"id\": \"clay\", \"title\": \"Clay\"}"),
                "{index}"
            );
            assert_eq!(
                std::fs::read(out.join("themes/clay/art/hero.png")).unwrap(),
                b"not drawn by gp"
            );
            assert!(
                read(&out.join("themes/clay/art.txt"))
                    .unwrap()
                    .starts_with("gp-art 1")
            );
            let cfg: serde_json::Value =
                serde_json::from_str(&read(&out.join("preview.json")).unwrap()).unwrap();
            assert_eq!(cfg["looks"], true);
        }
    }

    #[test]
    #[ignore = "needs the games repo beside this one (phase 1.3)"]
    fn a_shared_theme_is_offered_on_both_models() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("templates");
        let (a, b) = (root.join("a"), root.join("b"));
        std::fs::create_dir_all(&a).unwrap();
        // Its two broken themes stay in `a` and are skipped there.
        let rom = rivet_rodeo_with_broken_themes(&a);
        let paper = a.join("themes/paper/theme.toml");
        let text = read(&paper).unwrap();
        assert!(text.contains("for = [\"rivet_rodeo\"]"), "{text}");
        std::fs::write(
            &paper,
            text.replace(
                "for = [\"rivet_rodeo\"]",
                "for = [\"rivet_rodeo\", \"rivet_rodeo_b\"]",
            ),
        )
        .unwrap();
        std::fs::create_dir_all(b.join("themes/paper")).unwrap();
        for f in ["template.toml", "asset-map.toml", "profiles.toml"] {
            std::fs::copy(a.join(f), b.join(f)).unwrap();
        }
        let model = read(&a.join("model.toml")).unwrap();
        std::fs::write(
            b.join("model.toml"),
            model.replacen("name = \"rivet_rodeo\"", "name = \"rivet_rodeo_b\"", 1),
        )
        .unwrap();
        std::fs::write(
            b.join("themes/paper").join(gp_theme::same_as::SAME_AS_FILE),
            "theme = \"../../../a/themes/paper\"\n",
        )
        .unwrap();
        let mut looks = Vec::new();
        for t in [&a, &b] {
            let out = dir.path().join("out").join(t.file_name().unwrap());
            let (json, code) = result_json(lift_rom(&rom, t, &out));
            assert_eq!(code, 0, "{json}");
            let skipped: Vec<&str> = json["looks_skipped"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|s| s["id"].as_str())
                .collect();
            assert!(!skipped.contains(&"paper"), "{json}");
            let index = read(&out.join("themes/index.json")).unwrap();
            assert!(index.contains("\"paper\""), "{index}");
            let mut files = Vec::new();
            list(
                &out.join("themes/paper"),
                &out.join("themes/paper"),
                &mut files,
            )
            .unwrap();
            files.sort();
            let drawn: Vec<(String, Vec<u8>)> = files
                .iter()
                .map(|f| {
                    (
                        f.clone(),
                        std::fs::read(out.join("themes/paper").join(f)).unwrap(),
                    )
                })
                .collect();
            assert!(drawn.iter().any(|(f, _)| f == gp_present::looks::LOOK_FILE));
            looks.push(drawn);
        }
        assert_eq!(
            looks[0], looks[1],
            "the paper look is drawn the same for both"
        );

        // A pointer to a theme whose `for` doesn't name the model isn't
        // offered, and says why.
        std::fs::write(&paper, text).unwrap();
        let out = dir.path().join("out/b-not-for");
        let (json, code) = result_json(lift_rom(&rom, &b, &out));
        assert_eq!(code, 0, "{json}");
        let skipped = &json["looks_skipped"][0];
        assert_eq!(skipped["id"], "paper", "{json}");
        assert_eq!(skipped["reason"], NOT_READ);
        assert!(
            skipped["detail"]
                .as_str()
                .unwrap()
                .contains("is for rivet_rodeo, not `rivet_rodeo_b`"),
            "{skipped}"
        );
    }

    /// Original Edition's stages list names its levels m50 (50m), m75 and
    /// m100 after 25m; with the page playing main only, main plays the slot
    /// of each of them the model has a level for (m50 always; m75 since
    /// 2026-11-05), a slot with no level is left as the model plays it, and
    /// m50 stays in the model.
    #[test]
    #[ignore = "needs the games repo beside this one (phase 1.3)"]
    fn a_stage_whose_level_the_page_does_not_play_is_the_stand_ins() {
        let file =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games/donkey-kong/model.toml");
        let mut model = Model::parse(&read(&file).unwrap()).unwrap();
        assert_eq!(
            model.stages.as_ref().unwrap().order,
            ["main", "m50", "m75", "m100"]
        );
        let expected: Vec<String> = model
            .stages
            .as_ref()
            .unwrap()
            .order
            .iter()
            .map(|s| {
                if s != "main" && model.levels.contains_key(s) {
                    "main".to_owned()
                } else {
                    s.clone()
                }
            })
            .collect();
        assert_eq!(
            expected[1], "main",
            "m50 is a level, so main stands in for it"
        );
        play_only(&mut model, &["main".to_owned()]);
        assert_eq!(model.stages.as_ref().unwrap().order, expected);
        assert!(model.levels.contains_key("m50"));
        // A list that leaves the stand-in out changes nothing.
        let mut other = Model::parse(&read(&file).unwrap()).unwrap();
        play_only(&mut other, &["m50".to_owned()]);
        assert_eq!(
            other.stages.as_ref().unwrap().order,
            ["main", "m50", "m75", "m100"]
        );
    }

    /// A Donkey Kong (JU) ROM has 25m, 75m and 100m (template.toml's
    /// [[loads]]): Original Edition's 50m is a stage it passes over, not
    /// one 25m plays, and the order and the levels stay as they are.
    #[test]
    #[ignore = "needs the games repo beside this one (phase 1.3)"]
    fn a_stage_a_loaded_rom_has_no_level_for_is_skipped() {
        let file =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games/donkey-kong/model.toml");
        let mut model = Model::parse(&read(&file).unwrap()).unwrap();
        let levels = ["main", "m75", "m100"].map(str::to_owned);
        skip_missing(&mut model, &levels);
        let st = model.stages.as_ref().unwrap();
        assert_eq!(st.order, ["main", "m50", "m75", "m100"]);
        assert_eq!(st.skip, ["m50"]);
        assert!(model.levels.contains_key("m50"));
        assert!(gp_model::validate::validate(&model).is_empty());
    }
}
