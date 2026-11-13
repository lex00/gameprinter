//! Looks: themes drawn for one game ahead of play, for a runtime to find
//! when it runs (`gp_present::looks`; docs/formats/theme.md, "Themes found
//! at run time"). A look's folder holds the files of the build's `assets/`
//! that the theme changes, under the same names, and `look.txt`. The web
//! preview reads `themes/index.json` beside its bundle. Presentation only: the simulation never reads them.

use std::collections::BTreeMap;
use std::path::Path;

use gp_model::Model;
use gp_present::looks::{LOOK_FILE, Look, ORIGINAL};
use gp_sim::Spec;

/// A theme to draw as a look.
#[derive(Debug, Clone)]
pub struct LookSpec {
    pub id: String,
    pub title: String,
    /// The theme stack it draws: one theme, or none for `original`.
    pub theme: gp_theme::Resolved,
}

impl LookSpec {
    /// The look of the lifted art and the game's own sound.
    pub fn original() -> Self {
        Self {
            id: ORIGINAL.into(),
            title: "Original".into(),
            theme: gp_theme::Resolved::none(),
        }
    }

    /// A theme directory as a look of its own, for the model `model`: a
    /// theme whose `for` doesn't name it is refused.
    pub fn theme_for(dir: &Path, model: &str) -> Result<Self, String> {
        let l = Self::theme(dir)?;
        if let Some(t) = l.theme.layers.first() {
            gp_theme::stack::check_for(&t.theme, model)?;
        }
        Ok(l)
    }

    /// A theme directory as a look of its own. A folder holding
    /// `same_as.toml` is read from the folder it points at.
    pub fn theme(dir: &Path) -> Result<Self, String> {
        let l = gp_theme::stack::load(dir, gp_theme::Scope::Baked)?;
        Ok(Self {
            id: l.theme.id.clone(),
            title: l.theme.title.clone(),
            theme: gp_theme::Resolved { layers: vec![l] },
        })
    }
}

/// The id and title of the look a stack draws: its top theme's, or
/// `original` with none.
pub fn own_look(theme: Option<&gp_theme::Resolved>) -> (String, String) {
    theme
        .and_then(|t| t.layers.last())
        .map(|l| (l.theme.id.clone(), l.theme.title.clone()))
        .unwrap_or_else(|| (ORIGINAL.into(), "Original".into()))
}

/// The names of the files a look drew, and each file's bytes.
type Drawn = (Vec<String>, BTreeMap<String, Vec<u8>>);

/// What a look draws, as files relative to the assets folder: the
/// pictures and overlays, `present.txt`, and its sound roles' files.
pub fn draw(
    model: &Model,
    spec: &Spec,
    assets: &Path,
    cues: &gp_present::sound::Cues,
    look: &LookSpec,
    notes: &mut Vec<String>,
) -> Result<Drawn, String> {
    let art = crate::present::art_themed(model, spec, assets, &look.theme)?;
    let names: Vec<String> = art
        .present
        .pictures
        .iter()
        .map(|p| p.file.clone())
        .collect();
    let mut files = BTreeMap::new();
    files.insert("present.txt".to_owned(), art.present_text().into_bytes());
    for p in art
        .pictures
        .iter()
        .chain(art.theme.overlays.iter().map(|(_, p)| p))
    {
        files.insert(p.file.clone(), p.png.clone());
    }
    for (role, wav) in crate::sound::role_files(cues, &look.theme, notes) {
        files.insert(crate::sound::role_path(&role), wav);
    }
    Ok((names, files))
}

/// A look's folder: the files of `look` that differ from the build's own
/// (`own`, relative to the assets folder), and its manifest, as paths
/// relative to the assets folder (`themes/<id>/...`). A runtime swaps
/// pictures by number, so the look's picture `i` is kept under the
/// build's name for picture `i`; a look with another number of pictures
/// is refused.
pub fn folder(
    own_names: &[String],
    own: &BTreeMap<String, Vec<u8>>,
    look: &LookSpec,
    names: &[String],
    files: &BTreeMap<String, Vec<u8>>,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    if names.len() != own_names.len() {
        return Err(format!(
            "the look `{}` draws {} pictures where the build draws {}, so a runtime can't swap it in",
            look.id,
            names.len(),
            own_names.len()
        ));
    }
    let renamed: BTreeMap<&str, &str> = names
        .iter()
        .zip(own_names)
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let mut out = Vec::new();
    let mut manifest = Look {
        id: look.id.clone(),
        title: look.title.clone(),
        original: look.id == ORIGINAL,
        files: Vec::new(),
        own_sound: !look.theme.replaces_sound(),
        art: None,
    };
    let mut seen = std::collections::BTreeSet::new();
    for (rel, bytes) in files {
        let rel = renamed.get(rel.as_str()).copied().unwrap_or(rel.as_str());
        if !seen.insert(rel.to_owned()) {
            continue;
        }
        if own.get(rel) != Some(bytes) || rel.starts_with("sound/roles/") {
            manifest.files.push(rel.to_owned());
            out.push((format!("themes/{}/{rel}", look.id), bytes.clone()));
        }
    }
    out.push((
        format!("themes/{}/{LOOK_FILE}", look.id),
        manifest.write().into_bytes(),
    ));
    Ok(out)
}

/// The looks a template offers beside the one its stack draws: each
/// theme in its `themes/` folder (a folder with `theme.toml`, or with
/// `same_as.toml` pointing at another template's theme), and `original`
/// when the game's assets are lifted from a ROM and the stack draws a
/// theme. `own` is the id of the stack's look, which isn't offered again.
/// A theme whose `for` doesn't name `model` is an error.
pub fn offered(
    themes_dir: &Path,
    own: &str,
    lifted: bool,
    model: &str,
) -> Vec<Result<LookSpec, String>> {
    let mut out = Vec::new();
    if lifted && own != ORIGINAL {
        out.push(Ok(LookSpec::original()));
    }
    let Ok(rd) = std::fs::read_dir(themes_dir) else {
        return out;
    };
    let mut dirs: Vec<_> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| gp_theme::same_as::is_theme_dir(p))
        .collect();
    dirs.sort();
    for d in dirs {
        let l = LookSpec::theme_for(&d, model);
        if l.as_ref().is_ok_and(|l| l.id == own) {
            continue;
        }
        out.push(l);
    }
    out
}

/// Whether a template's assets come from a ROM (`[source] how = "rom"` in
/// its `template.toml`): its builds offer the look `original`.
pub fn lifted_from_rom(model_dir: &Path) -> bool {
    std::fs::read_to_string(model_dir.join("template.toml"))
        .ok()
        .and_then(|t| t.parse::<toml::Table>().ok())
        .and_then(|t| Some(t.get("source")?.get("how")?.as_str()? == "rom"))
        .unwrap_or(false)
}
