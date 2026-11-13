//! Writes what a web build of a model loads, for the playable web build
//! (docs/web.md):
//!
//!   gp-web-prep [--stack FILE] [--themes] [--theme DIR]... MODEL ASSETS OUT [BASELINE...]
//!
//! With theme options the pictures are drawn with that theme stack
//! (docs/formats/theme.md), and `preview.json` names the stack's overlay
//! and particles; the bundle and the hashes don't change.
//!
//! OUT gets `bundle.json` (the model bundle the browser interpreter loads),
//! `assets/` (the generator's pictures and `present.txt`, the
//! presentation),
//! `preview.json` (what the preview page draws them with, and the spec
//! digest every build of the model must report, and the model's switches
//! by group), `profiles.toml` (the model's, which the page applies to the
//! bundle for any set of switches) and `profiles/<name>.json` (the model
//! bundle in each preset; docs/model.md, "Profiles") and, for each
//! baseline, `runs/<name>.frames` (its input as a frames file) and
//! `runs/<name>.hashes` (the native interpreter's state hash at power-on
//! and after every frame), with `runs/index.json` naming them.
//!
//! Exit codes: 0 written, 2 couldn't run.

use std::path::{Path, PathBuf};

use gp_model::Model;

fn main() {
    if let Err(e) = run() {
        eprintln!("gp-web-prep: {e}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), String> {
    let mut args: Vec<PathBuf> = Vec::new();
    let (mut stack, mut themes, mut theme_dirs) = (None, false, Vec::new());
    let mut it = std::env::args_os().skip(1);
    while let Some(a) = it.next() {
        match a.to_str() {
            Some("--stack") => stack = Some(PathBuf::from(it.next().ok_or("--stack FILE")?)),
            Some("--themes") => themes = true,
            Some("--theme") => theme_dirs.push(PathBuf::from(it.next().ok_or("--theme DIR")?)),
            _ => args.push(PathBuf::from(a)),
        }
    }
    let [model_arg, assets, out, baselines @ ..] = args.as_slice() else {
        return Err(
            "usage: gp-web-prep [--stack FILE] [--themes] [--theme DIR]... MODEL ASSETS OUT [BASELINE...]"
                .into(),
        );
    };
    let file = if model_arg.is_dir() {
        model_arg.join(gp_model::MODEL_FILE)
    } else {
        model_arg.clone()
    };
    let name = model_name(&file)?;
    let theme = theme_stack(model_arg, &name, stack, themes, &theme_dirs)?;
    let home = gp_evidence::Paths::from_env().ok().map(|p| p.home);
    // The shared code (gp-web-lift's, which the public page's in-browser
    // lift runs too), so both write the same files.
    // The other looks the preview's settings offer: each of the
    // template's themes, and the original art where it was lifted.
    let model_dir = file.parent().unwrap_or(Path::new("."));
    let lifted = gp_look::looks::lifted_from_rom(model_dir);
    let own = gp_look::looks::own_look((!theme.is_empty()).then_some(&theme));
    let mut looks = Vec::new();
    for l in gp_look::looks::offered(&model_dir.join("themes"), &own.0, lifted, &name) {
        match l {
            Ok(l) => looks.push(l),
            Err(e) => eprintln!("gp-web-prep: a theme beside the model isn't offered: {e}"),
        }
    }
    let prepped = gp_web_lift::prep(&file, assets, out, &theme, home.as_deref(), &looks, lifted)?;
    let bundle = prepped.bundle;
    catalogue(model_dir, out, home.as_deref())?;

    let mut names = Vec::new();
    for b in baselines {
        let log = gp_evidence::baseline::Baseline::load(b)
            .map_err(|e| format!("{}: {e}", b.display()))?
            .input()
            .map_err(|e| format!("{}: {e}", b.display()))?;
        let mut frames = Vec::new();
        for f in &log.frames {
            let word = f.buttons_word();
            if word > 0xFF {
                return Err(format!("{}: a model reads controller 1 only", b.display()));
            }
            frames.push((word as u8, f.reset));
        }
        let name = b
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .ok_or_else(|| format!("{}: no file name", b.display()))?;
        let frames_text = gp_look::frames_file(&frames);
        let mut p = gp_web::Preview::load(&bundle.to_bytes())?;
        let hashes = p.run_frames(&frames_text)?;
        write(
            &out.join("runs").join(format!("{name}.frames")),
            frames_text.as_bytes(),
        )?;
        write(
            &out.join("runs").join(format!("{name}.hashes")),
            hashes.as_bytes(),
        )?;
        names.push(name);
    }
    write(
        &out.join("runs").join("index.json"),
        serde_json::json!(names).to_string().as_bytes(),
    )?;
    println!("wrote {} ({} runs)", out.display(), names.len());
    Ok(())
}

/// The theme stack the flags name: `--themes` is the model's own stack,
/// `--stack` a stack file, each `--theme` a personal layer on top.
fn theme_stack(
    model_arg: &Path,
    name: &str,
    stack: Option<PathBuf>,
    themes: bool,
    dirs: &[PathBuf],
) -> Result<gp_theme::Resolved, String> {
    use gp_theme::stack::{Places, load, resolve};
    let model_dir = if model_arg.is_dir() {
        model_arg.to_owned()
    } else {
        model_arg.parent().map(Path::to_owned).unwrap_or_default()
    };
    let home = std::env::var_os("GP_HOME").map(PathBuf::from);
    let places = Places::for_model(&model_dir, home.as_deref());
    let mut s = gp_theme::Stack {
        format: gp_theme::FORMAT_VERSION.into(),
        model: None,
        layer: Vec::new(),
    };
    if themes {
        s.layer.extend(places.default_stack(name)?.layer);
    }
    if let Some(f) = stack {
        let t = read(&f)?;
        s.layer.extend(
            gp_theme::Stack::parse(&t)
                .map_err(|e| format!("{}: {e}", f.display()))?
                .layer,
        );
    }
    let mut r = resolve(&s, &places, name)?;
    for d in dirs {
        r.layers.push(load(d, gp_theme::Scope::Personal)?);
    }
    Ok(r)
}

/// The model's `model.name`, which its theme stack is found by.
fn model_name(file: &Path) -> Result<String, String> {
    let text = read(file)?;
    let model = Model::parse(&text).map_err(|e| format!("{}: {e}", file.display()))?;
    Ok(model.model.name)
}

fn read(p: &Path) -> Result<String, String> {
    std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))
}

fn write(p: &Path, bytes: &[u8]) -> Result<(), String> {
    gp_web_lift::write(p, bytes)
}

/// Adds `catalogue` to preview.json: the game's id (its template's) and the
/// read URLs of the package mirrors (mirrors.toml in $GP_HOME and in the
/// repo, docs/packages.md), where the page looks for the game's themes
/// index (docs/theme-publishing.md, "In the game").
fn catalogue(model_dir: &Path, out: &Path, home: Option<&Path>) -> Result<(), String> {
    let table = |p: &Path| -> Option<toml::Table> { std::fs::read_to_string(p).ok()?.parse().ok() };
    let game = table(&model_dir.join("template.toml"))
        .and_then(|t| Some(t.get("template")?.get("id")?.as_str()?.to_owned()))
        .or_else(|| {
            model_dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
        })
        .unwrap_or_default();
    let repo = gp_evidence::paths::enclosing_git_work_tree(model_dir);
    let mut mirrors: Vec<String> = Vec::new();
    for f in [
        home.map(|h| h.join("mirrors.toml")),
        repo.map(|r| r.join("mirrors.toml")),
    ]
    .into_iter()
    .flatten()
    {
        for m in table(&f)
            .and_then(|t| t.get("mirror")?.as_array().cloned())
            .unwrap_or_default()
        {
            if let Some(r) = m.get("read").and_then(|r| r.as_str())
                && !mirrors.iter().any(|x| x == r)
            {
                mirrors.push(r.to_owned());
            }
        }
    }
    let path = out.join("preview.json");
    let mut v: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?,
    )
    .map_err(|e| e.to_string())?;
    v["catalogue"] = serde_json::json!({ "game": game, "mirrors": mirrors });
    write(&path, v.to_string().as_bytes())
}
