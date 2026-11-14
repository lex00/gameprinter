//! `gp assets open`: the round trip with an external pixel editor
//! (docs/sprite-editing.md, phase 3). It writes an entity's sprite set for
//! the editor in the layout the image format config gives
//! (docs/formats/image-config.md, through `gp-imagefmt`), launches the
//! editor on it when it's installed (never downloading it), then watches
//! the files: each save is read back with every import check and proposed
//! as an `assets_edit` with every edit check, and placed with provenance
//! when accepted.
//!
//! Pixelorama doesn't reload a file changed on disk, and its command line
//! only exports, so the trip is files: the person saves PNGs over the
//! exported ones, and this command notices by their digest.
//!
//! Ported from game-printer's assets_open.rs, with `image_layers` (the
//! image config's layers) moved here from its sprites.rs. An arcade game's lift is refused in
//! words: the editor trip cuts frames into NES tiles (gp_sprites::chr).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use clap::Args;
use gp_evidence::Paths;
use serde_json::{Value, json};

use crate::Outcome;

#[derive(Args, Clone, Debug)]
pub struct OpenArgs {
    /// A working copy's folder (holding model.toml), or a template under
    /// templates/ to fork into the working copies first.
    pub copy: String,
    /// The entity whose sprite set to edit.
    pub entity: String,
    /// The tiles slot (default `tiles`, or the entity's sprite's).
    #[arg(long)]
    pub slot: Option<String>,
    /// The editor: `pixelorama` (the default), `libresprite`, `png` (write
    /// the files only). The format follows the image config; this sets its
    /// `editor` key for the trip.
    #[arg(long, default_value = "pixelorama")]
    pub editor: String,
    /// An image config layer over the others (docs/formats/image-config.md).
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// Where working copies live (default $GP_HOME/work/mcp, as gp mcp's).
    #[arg(long)]
    pub work: Option<PathBuf>,
    /// The repo, for templates and baselines (default: the enclosing one).
    #[arg(long)]
    pub repo: Option<PathBuf>,
    /// Lifted assets, instead of $GP_HOME/work/assets/<source>.
    #[arg(long)]
    pub assets: Option<PathBuf>,
    /// The folder to write (default <copy>/exchange/<entity>).
    #[arg(long)]
    pub out: Option<PathBuf>,
    /// Place each save at once (still undoable) instead of proposing it.
    #[arg(long)]
    pub place: bool,
    /// Write the files and don't start the editor.
    #[arg(long)]
    pub no_launch: bool,
    /// Stop after the first save is brought back.
    #[arg(long)]
    pub once: bool,
    /// Stop watching after this many seconds (0: until interrupted).
    #[arg(long, default_value_t = 0)]
    pub timeout: u64,
    /// How often to look for a save, in milliseconds.
    #[arg(long, default_value_t = 500)]
    pub poll_ms: u64,
}

/// The config layers, lowest first: $GP_HOME/config/image.toml, the
/// project's gp.toml `[image]` (the first found from the current directory
/// up), then the file given.
pub(crate) fn image_layers(
    paths: &Paths,
    file: Option<&Path>,
) -> Result<Vec<gp_imagefmt::config::Layer>> {
    use gp_imagefmt::config::Layer;
    let mut layers = Vec::new();
    let home = paths.home.join("config").join("image.toml");
    if let Ok(t) = std::fs::read_to_string(&home) {
        layers.push(Layer::parse(&t).map_err(|e| anyhow::anyhow!("{}: {e}", home.display()))?);
    }
    let mut dir = std::env::current_dir().ok();
    while let Some(d) = dir {
        let f = d.join("gp.toml");
        if let Ok(t) = std::fs::read_to_string(&f) {
            let table: toml::Table = toml::from_str(&t).with_context(|| f.display().to_string())?;
            if table.contains_key("image") {
                layers.push(Layer::parse(&t).map_err(|e| anyhow::anyhow!("{}: {e}", f.display()))?);
            }
            break;
        }
        dir = d.parent().map(Path::to_path_buf);
    }
    if let Some(f) = file {
        let t = std::fs::read_to_string(f).with_context(|| format!("reading {}", f.display()))?;
        layers.push(Layer::parse(&t).map_err(|e| anyhow::anyhow!("{}: {e}", f.display()))?);
    }
    Ok(layers)
}

/// Where an editor is installed, looked for without installing anything:
/// `GP_<EDITOR>` (a path), the PATH, then the usual application folders.
pub fn find_editor(editor: &str) -> Option<PathBuf> {
    let var = std::env::var_os(format!("GP_{}", editor.to_uppercase())).map(PathBuf::from);
    find_in(
        editor,
        var,
        std::env::var_os("PATH"),
        std::env::var_os("HOME").map(PathBuf::from),
    )
}

/// [`find_editor`] with its environment given.
fn find_in(
    editor: &str,
    named: Option<PathBuf>,
    path: Option<std::ffi::OsString>,
    home: Option<PathBuf>,
) -> Option<PathBuf> {
    if let Some(p) = named {
        return p.is_file().then_some(p);
    }
    let names: &[&str] = match editor {
        "pixelorama" => &["pixelorama", "Pixelorama"],
        "libresprite" => &["libresprite", "LibreSprite"],
        _ => return None,
    };
    if let Some(path) = path {
        for dir in std::env::split_paths(&path) {
            for n in names {
                let p = dir.join(n);
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }
    let home = home.unwrap_or_default();
    let apps: Vec<PathBuf> = match editor {
        "pixelorama" => vec![
            PathBuf::from("/Applications/Pixelorama.app/Contents/MacOS/Pixelorama"),
            home.join("Applications/Pixelorama.app/Contents/MacOS/Pixelorama"),
            PathBuf::from("/usr/bin/pixelorama"),
            PathBuf::from("/usr/local/bin/pixelorama"),
        ],
        _ => vec![
            PathBuf::from("/Applications/LibreSprite.app/Contents/MacOS/libresprite"),
            home.join("Applications/LibreSprite.app/Contents/MacOS/libresprite"),
            PathBuf::from("/usr/bin/libresprite"),
        ],
    };
    apps.into_iter().find(|p| p.is_file())
}

fn config(a: &OpenArgs, paths: &Paths) -> Result<gp_mcp::Config> {
    let cwd = std::env::current_dir()?;
    let repo = match &a.repo {
        Some(r) => std::path::absolute(r)?,
        None => gp_evidence::paths::enclosing_git_work_tree(&cwd)
            .ok_or_else(|| anyhow!("not in a git work tree; give the repo with --repo"))?,
    };
    let work = match &a.work {
        Some(w) => std::path::absolute(w)?,
        None => paths.home.join("work").join("mcp"),
    };
    Ok(gp_mcp::Config {
        gp: std::env::current_exe().context("finding the gp binary")?,
        repo,
        work,
        home: paths.home.clone(),
        rom: None,
        assets: a.assets.as_deref().map(std::path::absolute).transpose()?,
        games: crate::roms_cmd::games_dir(None),
    })
}

/// The working copy: the folder given, or a fork of the template (kept
/// when it exists, so a second open edits the same copy).
fn copy_of(a: &OpenArgs, config: &mut gp_mcp::Config) -> Result<(String, String)> {
    let p = Path::new(&a.copy);
    if p.join(gp_model::MODEL_FILE).is_file() {
        let dir = std::path::absolute(p)?;
        let name = dir
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| anyhow!("{} has no name", dir.display()))?
            .to_owned();
        config.work = dir
            .parent()
            .ok_or_else(|| anyhow!("{} has no parent", dir.display()))?
            .to_path_buf();
        let model = crate::mechanics::load_model(&dir)?;
        let template = model.model.source.clone().unwrap_or(model.model.name);
        return Ok((name, template));
    }
    let name = a.copy.replace('-', "_");
    if !config.work.join(&name).join(gp_model::MODEL_FILE).is_file() {
        std::fs::create_dir_all(&config.work)?;
        let r = gp_mcp::tools::call(
            config,
            "template_fork",
            json!({ "template": a.copy, "copy": name }),
        )
        .ok_or_else(|| anyhow!("no template_fork tool"))?;
        if r.is_error {
            bail!("forking {}: {}", a.copy, r.json["error"]);
        }
    }
    Ok((name, a.copy.clone()))
}

pub(crate) fn open(a: &OpenArgs, paths: &Paths) -> Result<Outcome> {
    let mut config = config(a, paths)?;
    let (copy, template) = copy_of(a, &mut config)?;
    let dir = config.work.join(&copy);
    let model = crate::mechanics::load_model(&dir)?;
    let lifted = match &config.assets {
        Some(p) => p.clone(),
        None => paths
            .home
            .join("work")
            .join("assets")
            .join(model.model.source.as_deref().unwrap_or(&model.model.name)),
    };
    if std::fs::read_to_string(lifted.join(gp_assets::MAP_FILE))
        .is_ok_and(|t| gp_assets::arcade::is_arcade(&t))
    {
        bail!(
            "{} is an arcade lift; the editor trip cuts frames into NES tiles (gp_sprites::chr), so an arcade game's sprites aren't opened yet",
            lifted.display()
        );
    }
    let slot = match &a.slot {
        Some(s) => s.clone(),
        None => model
            .entities
            .get(&a.entity)
            .and_then(|e| e.core.sprite.as_ref())
            .map(|s| s.slot.clone())
            .ok_or_else(|| anyhow!("`{}` has no sprite", a.entity))?,
    };
    let layers = image_layers(paths, a.config.as_deref())?;
    let mut cfg = gp_imagefmt::config::resolve(&layers, Some(&template), Some(&a.entity))
        .map_err(anyhow::Error::msg)?;
    use gp_imagefmt::config::{Editor, PaletteFile};
    match a.editor.as_str() {
        "pixelorama" => {
            cfg.editor = Editor::Pixelorama;
            // Pixelorama loads its own palette format directly.
            if cfg.palette_file == PaletteFile::Gpl {
                cfg.palette_file = PaletteFile::Pixelorama;
            }
        }
        "libresprite" | "aseprite" => cfg.editor = Editor::Aseprite,
        "png" => cfg.editor = Editor::Png,
        e => bail!("--editor is pixelorama, libresprite or png, not {e}"),
    }
    let out = a
        .out
        .clone()
        .unwrap_or_else(|| dir.join("exchange").join(&a.entity));
    let (trip, files) = gp_mcp::editor_trip::export(&config, &copy, &a.entity, &slot, &cfg, &out)
        .map_err(anyhow::Error::msg)?;
    let image = out.join(trip.images.first().cloned().unwrap_or_default());
    eprintln!(
        "{} frames of {} written to {} ({} files)",
        trip.frames.len(),
        a.entity,
        out.display(),
        files.len()
    );

    // The editor, if it's installed. Game Printer never downloads one.
    let mut launched = Value::Null;
    if !a.no_launch && a.editor != "png" {
        match find_editor(&a.editor) {
            Some(exe) => {
                let child = std::process::Command::new(&exe)
                    .arg(&image)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn();
                match child {
                    Ok(c) => {
                        eprintln!("opened {} in {}", image.display(), exe.display());
                        launched = json!({ "editor": exe, "pid": c.id() });
                    }
                    Err(e) => eprintln!("couldn't start {}: {e}", exe.display()),
                }
            }
            None => eprintln!(
                "{} isn't installed here (looked for GP_{}, the PATH and the usual application folders); open {} in it yourself. Game Printer doesn't download editors.",
                a.editor,
                a.editor.to_uppercase(),
                image.display()
            ),
        }
    }

    // Watch the images for saves.
    let started = Instant::now();
    let mut last = gp_mcp::editor_trip::images_digest(&out, &trip);
    let mut saves = Vec::new();
    eprintln!("watching {} for saves", out.display());
    loop {
        if a.timeout > 0 && started.elapsed() >= Duration::from_secs(a.timeout) {
            break;
        }
        std::thread::sleep(Duration::from_millis(a.poll_ms.max(50)));
        let now = gp_mcp::editor_trip::images_digest(&out, &trip);
        if now == last {
            continue;
        }
        // Let a save finish writing before reading it.
        std::thread::sleep(Duration::from_millis(a.poll_ms.clamp(50, 500)));
        let settled = gp_mcp::editor_trip::images_digest(&out, &trip);
        if settled != now {
            continue;
        }
        last = settled;
        let r = gp_mcp::editor_trip::propose_save(&config, &out, a.place, Some("person"))
            .map_err(anyhow::Error::msg)?;
        let mut j = r.json.clone();
        if let Some(o) = j.as_object_mut() {
            for k in ["preview", "in_context", "contexts", "readiness", "after"] {
                o.remove(k);
            }
        }
        eprintln!(
            "save {}: {}",
            saves.len() + 1,
            if r.is_error {
                format!(
                    "refused: {}",
                    j.get("problems").or(j.get("error")).unwrap_or(&Value::Null)
                )
            } else {
                format!(
                    "edit {} {} ({} pixels)",
                    j["seq"],
                    j["status"].as_str().unwrap_or(""),
                    j["pixels_changed"]
                )
            }
        );
        saves.push(json!({ "refused": r.is_error, "result": j }));
        if a.once {
            break;
        }
    }
    let text = format!("{} saves brought back from {}", saves.len(), out.display());
    Ok(Outcome::ok(
        json!({
            "copy": copy, "work": config.work, "entity": a.entity, "slot": slot, "out": out,
            "files": files, "image": image, "launched": launched, "config": cfg, "saves": saves,
        }),
        text,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_editor_named_by_its_variable_is_found_and_never_fetched() {
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("fake-pixelorama");
        std::fs::write(&fake, "#!/bin/sh\n").unwrap();
        assert_eq!(
            find_in("pixelorama", Some(fake.clone()), None, None),
            Some(fake)
        );
        assert_eq!(
            find_in("pixelorama", Some(dir.path().join("none")), None, None),
            None,
            "a named path that isn't there"
        );
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("libresprite"), "").unwrap();
        assert_eq!(
            find_in(
                "libresprite",
                None,
                Some(bin.clone().into_os_string()),
                None
            ),
            Some(bin.join("libresprite"))
        );
        assert_eq!(
            find_in("mspaint", None, Some(bin.into_os_string()), None),
            None
        );
    }
}
