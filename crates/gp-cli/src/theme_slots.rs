//! `gp theme check`'s slot names (docs/formats/theme.md, "Slot names"):
//! each layer's restyles against every model in its `for` list, read from
//! the templates of the same game (`template.game`), or against the model
//! checked when it has no `for`.

use std::fs;
use std::path::{Path, PathBuf};

use gp_model::Model;
use gp_theme::Theme;
use gp_theme::slots::{self, Finding, ModelSlots};
use serde_json::{Value, json};

/// The slot check of a stack's layers.
pub(crate) struct SlotCheck {
    /// One line a finding, and a line for each model in `for` not found.
    pub text: String,
    pub json: Value,
    pub refused: bool,
}

/// Checks each layer's theme (a stack's, bottom first) for the model `m`
/// in the template `dir`.
pub(crate) fn check(dir: &Path, m: &Model, themes: &[&Theme]) -> SlotCheck {
    let siblings = same_game(dir);
    let mut text = String::new();
    let mut layers = Vec::new();
    let mut refused = false;
    for theme in themes {
        let names: Vec<String> = if theme.for_models.is_empty() {
            vec![m.model.name.clone()]
        } else {
            theme.for_models.clone()
        };
        let mut models = Vec::new();
        let mut missing = Vec::new();
        for n in &names {
            if *n == m.model.name {
                models.push(slots_of(dir, m));
            } else if let Some((d, other)) = siblings.iter().find(|(_, o)| o.model.name == *n) {
                models.push(slots_of(d, other));
            } else {
                missing.push(n.clone());
            }
        }
        let found = slots::check(theme, &models);
        let checked: Vec<&str> = models.iter().map(|x| x.model.as_str()).collect();
        let bad = found.iter().filter(|f| f.refused).count();
        text.push_str(&format!(
            "slots of `{}` against {}: {} restyles, {bad} refused, {} notes\n",
            theme.id,
            checked.join(", "),
            theme.restyle.iter().filter(|r| r.slot.is_some()).count(),
            found.len() - bad
        ));
        for f in &found {
            let mark = if f.refused { "REFUSED " } else { "note: " };
            text.push_str(&format!("  {mark}{}\n", f.message));
        }
        for n in &missing {
            text.push_str(&format!(
                "  note: `{n}` is in `for`, but no template of this game here has it; not checked\n"
            ));
        }
        refused |= bad > 0;
        layers.push(json!({
            "theme": theme.id,
            "models": checked,
            "not_found": missing,
            "findings": found.iter().map(Finding::clone).collect::<Vec<_>>(),
        }));
    }
    SlotCheck {
        text,
        json: json!(layers),
        refused,
    }
}

/// The other templates in `dir`'s parent with the same `template.game`,
/// with their models.
pub(crate) fn same_game(dir: &Path) -> Vec<(PathBuf, Model)> {
    let Some(game) = game_of(dir) else {
        return Vec::new();
    };
    let (Some(parent), Ok(me)) = (dir.parent(), dir.canonicalize()) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = fs::read_dir(parent)
        .map(|d| d.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    dirs.sort();
    dirs.into_iter()
        .filter(|d| d.canonicalize().is_ok_and(|c| c != me))
        .filter(|d| game_of(d).as_deref() == Some(game.as_str()))
        .filter_map(|d| {
            let (m, _) = crate::theme::read_model(&d).ok()?;
            Some((d, m))
        })
        .collect()
}

/// A template's `[template] game`.
pub(crate) fn game_of(dir: &Path) -> Option<String> {
    let text = fs::read_to_string(dir.join("template.toml")).ok()?;
    let t: toml::Table = text.parse().ok()?;
    t.get("template")?.get("game")?.as_str().map(str::to_owned)
}

/// A model's slots, with the entries its asset map lists for each slot's
/// source asset when the template names a map.
fn slots_of(dir: &Path, m: &Model) -> ModelSlots {
    let mut s = ModelSlots::of(m);
    let (_, _, map_file) = crate::theme_kit::manifest(dir);
    let map = map_file
        .and_then(|f| fs::read_to_string(f).ok())
        .and_then(|t| t.parse::<toml::Table>().ok());
    let Some(assets) = map
        .as_ref()
        .and_then(|t| t.get("asset")?.as_array().cloned())
    else {
        return s;
    };
    for (name, slot) in &m.assets {
        let Some(from) = &slot.from else { continue };
        let entries = assets
            .iter()
            .find(|a| a.get("name").and_then(|n| n.as_str()) == Some(from.asset.as_str()))
            .and_then(|a| a.get("entry")?.as_array().cloned())
            .unwrap_or_default();
        s.add_entries(
            name,
            entries
                .iter()
                .filter_map(|e| e.get("name")?.as_str().map(str::to_owned)),
        );
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use gp_theme::Scope;

    /// A made-up theme over the repo's Donkey Kong models (no ROM data:
    /// model.toml and the asset maps only).
    fn made_up(for_models: &str, restyles: &[(&str, Option<&str>)]) -> Theme {
        let mut text = format!(
            "format = \"0.1.3\"\nid = \"made-up\"\ntitle = \"Made up\"\nlicense = \"CC0-1.0\"\nfor = [{for_models}]\n"
        );
        for (slot, entry) in restyles {
            text.push_str(&format!(
                "[[restyle]]\nslot = \"{slot}\"\n{}file = \"x.pix\"\norigin = \"drawn\"\nlicense = \"CC0-1.0\"\nsha256 = \"00\"\n",
                entry.map(|e| format!("entry = \"{e}\"\n")).unwrap_or_default()
            ));
        }
        toml::from_str(&text).unwrap()
    }

    /// Donkey Kong in the games checkout; None (the test passes over it)
    /// without one.
    fn dk() -> Option<(PathBuf, Model)> {
        let d = crate::theme_template::test_games()?.join("donkey-kong");
        let (m, _) = crate::theme::read_model(&d).unwrap();
        Some((d, m))
    }

    #[test]
    fn a_misspelt_slot_is_refused_with_the_nearest_name() {
        let Some((d, m)) = dk() else { return };
        let t = made_up("\"donkey_kong_oe\"", &[("sprite_tile", None)]);
        let c = check(&d, &m, &[&t]);
        assert!(c.refused, "{}", c.text);
        assert!(
            c.text.contains(
                "REFUSED restyle 1 (x.pix): no slot `sprite_tile` in donkey_kong_oe; the nearest is `sprite_tiles`"
            ),
            "{}",
            c.text
        );
    }

    #[test]
    fn a_misspelt_entry_is_refused_with_the_nearest_entry() {
        let Some((d, m)) = dk() else { return };
        let t = made_up("", &[("stage_screens", Some("screen_25n"))]);
        let c = check(&d, &m, &[&t]);
        assert!(c.refused, "{}", c.text);
        assert!(c.text.contains("the nearest is `screen_25m`"), "{}", c.text);
    }

    #[test]
    fn without_for_only_the_given_model_is_checked() {
        let Some((d, m)) = dk() else { return };
        let c = check(&d, &m, &[&made_up("", &[("title_tile", None)])]);
        assert!(c.refused, "{}", c.text);
        assert!(c.text.contains("against donkey_kong_oe:"), "{}", c.text);
    }

    #[test]
    fn every_templates_themes_pass() {
        // gameprinter: its own templates' themes (Moss Hop) and the games
        // checkout's, whose Donkey Kong has its kit only since its paper
        // theme was taken out (2026-11-11).
        let dirs = crate::theme_template::test_game_dirs();
        let mut seen = 0;
        for d in dirs {
            let Ok(themes) = fs::read_dir(d.join("themes")) else {
                continue;
            };
            let (m, _) = crate::theme::read_model(&d).unwrap();
            for t in themes.filter_map(|e| e.ok().map(|e| e.path())) {
                // A same_as.toml pointer is checked as its target, for
                // this template's model.
                if !gp_theme::same_as::is_theme_dir(&t) {
                    continue;
                }
                let loaded = gp_theme::stack::load(&t, Scope::Baked).unwrap();
                let c = check(&d, &m, &[&loaded.theme]);
                assert!(!c.refused, "{}: {}", t.display(), c.text);
                seen += 1;
            }
        }
        assert!(seen >= 3, "{seen} themes: Moss Hop's crt, dusk and paper");
    }
}
