//! A theme kit's prompt words (docs/theme-kit.md, "Prompt words"): the
//! file `templates/GAME/themes/kit/words.toml`, written once per game by a
//! person or an agent who has watched the template's clips. It gives each
//! art slot a short neutral label, a description in plain function words
//! and a pose phrase for each frame, and each sound role a description, in
//! place of the words the kit takes from the model. It also lists the
//! words no prompt-facing text of the kit may hold: the original's names
//! and its iconic things (Donkey Kong's barrels and girders). The game's
//! own terms stay in each entry's `note`, which never reaches the kit.
//!
//! `gp theme kit` writes the kit with the words applied, `gp theme sheet`
//! puts the labels and poses on its grids and in its prompts, and
//! `gp template check` and this module's test refuse a kit whose
//! prompt-facing text holds a denied word.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::theme_kit::Kit;

/// The words file, beside kit.toml.
pub const WORDS_FILE: &str = "words.toml";

/// A game's prompt words.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Words {
    pub format: String,
    /// Words (or phrases of several words) no prompt-facing text may hold,
    /// matched whole and without case, with an `s` or `es` plural.
    #[serde(default)]
    pub deny: Vec<String>,
    #[serde(default)]
    pub art: Vec<ArtWords>,
    #[serde(default)]
    pub sound: Vec<SoundWords>,
}

/// The words for one art slot.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtWords {
    /// The kit's slot id.
    pub id: String,
    /// A short neutral name, printed on a sheet's cells.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// The prompt-facing description, in place of the model's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// One plain phrase a frame, in the kit's frame order.
    #[serde(default)]
    pub poses: Vec<String>,
    /// For people only: the game's own term. Never copied into the kit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// The words for one sound role.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SoundWords {
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// A template's words file, when it has one.
pub fn load(template: &Path) -> Result<Option<Words>> {
    let f = template.join("themes").join("kit").join(WORDS_FILE);
    if !f.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(&f).with_context(|| format!("reading {}", f.display()))?;
    let w: Words = toml::from_str(&text).with_context(|| format!("{}", f.display()))?;
    Ok(Some(w))
}

/// Applies the words to a kit: labels, descriptions, poses and sound
/// descriptions. Returns what doesn't fit (an id the kit hasn't, a pose
/// count that isn't the slot's frame count).
pub fn apply(kit: &mut Kit, w: &Words) -> Vec<String> {
    let mut problems = Vec::new();
    for aw in &w.art {
        let Some(a) = kit.art.iter_mut().find(|a| a.id == aw.id) else {
            problems.push(format!(
                "words for `{}`, which the kit has no slot for",
                aw.id
            ));
            continue;
        };
        if let Some(l) = &aw.label {
            a.label.clone_from(l);
        }
        if let Some(d) = &aw.description {
            a.description.clone_from(d);
        }
        if !aw.poses.is_empty() {
            if aw.poses.len() != a.frames as usize {
                problems.push(format!(
                    "`{}` has {} frames and {} poses",
                    aw.id,
                    a.frames,
                    aw.poses.len()
                ));
            }
            a.poses.clone_from(&aw.poses);
        }
    }
    for sw in &w.sound {
        let Some(s) = kit.sound.iter_mut().find(|s| s.role == sw.role) else {
            problems.push(format!(
                "words for sound `{}`, which the kit hasn't",
                sw.role
            ));
            continue;
        };
        if let Some(d) = &sw.description {
            s.description.clone_from(d);
        }
    }
    problems
}

/// The kit's prompt-facing text, with where each is: every art slot's
/// label, description and poses, and every sound role's description. Ids,
/// timing, cues and anchors are keys and rules, never written into a
/// prompt or onto a sheet.
pub fn prompt_texts(kit: &Kit) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for a in &kit.art {
        out.push((format!("{} label", a.id), a.label.clone()));
        out.push((format!("{} description", a.id), a.description.clone()));
        for (k, p) in a.poses.iter().enumerate() {
            out.push((format!("{} pose {k}", a.id), p.clone()));
        }
    }
    for s in &kit.sound {
        out.push((
            format!("sound {} description", s.role),
            s.description.clone(),
        ));
    }
    out
}

/// Lowercase words of a text: letters and digits, anything else a break.
fn tokens(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The denied words a text holds, each once.
pub fn denied(text: &str, deny: &[String]) -> Vec<String> {
    let words = tokens(text);
    let mut out = Vec::new();
    for d in deny {
        let phrase = tokens(d);
        let Some((last, head)) = phrase.split_last() else {
            continue;
        };
        let n = phrase.len();
        let hit = words.windows(n).any(|w| {
            w[..n - 1] == *head
                && (w[n - 1] == *last
                    || w[n - 1] == format!("{last}s")
                    || w[n - 1] == format!("{last}es"))
        });
        if hit {
            out.push(d.clone());
        }
    }
    out
}

/// Each prompt-facing text of a kit that holds a denied word.
pub fn check(kit: &Kit, deny: &[String]) -> Vec<String> {
    prompt_texts(kit)
        .into_iter()
        .filter_map(|(at, text)| {
            let d = denied(&text, deny);
            (!d.is_empty()).then(|| format!("{at} holds {}: \"{text}\"", d.join(", ")))
        })
        .collect()
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn a_denied_word_is_found_whole_and_in_the_plural() {
        let deny = vec!["barrel".to_owned(), "oil drum".to_owned()];
        assert_eq!(denied("who throws the Barrels", &deny), vec!["barrel"]);
        assert_eq!(
            denied("a fire out of the oil drum", &deny),
            vec!["oil drum"]
        );
        assert!(denied("a barrelhouse tune; oil, then a drum", &deny).is_empty());
    }

    /// Every template with a words file: its kit holds the words (the
    /// poses, one a frame) and no prompt-facing text holds a denied word.
    #[test]
    fn templates_kits_hold_no_denied_word() {
        // gameprinter: its own templates (game-printer's had every game's).
        let mut seen = 0;
        for dir in crate::theme_template::test_own_templates() {
            let Some(w) = load(&dir).unwrap() else {
                continue;
            };
            seen += 1;
            let kit_file = dir.join("themes/kit").join(crate::theme_kit::KIT_FILE);
            let kit: Kit = toml::from_str(&fs::read_to_string(&kit_file).unwrap()).unwrap();
            let bad = check(&kit, &w.deny);
            assert!(bad.is_empty(), "{}: {bad:#?}", kit_file.display());
            for aw in &w.art {
                let a = kit
                    .art
                    .iter()
                    .find(|a| a.id == aw.id)
                    .unwrap_or_else(|| panic!("{}: no slot {}", dir.display(), aw.id));
                assert_eq!(
                    a.poses,
                    aw.poses,
                    "{}: {}'s poses aren't the words file's; run gp theme kit",
                    dir.display(),
                    aw.id
                );
                if !aw.poses.is_empty() {
                    assert_eq!(
                        aw.poses.len() as u32,
                        a.frames,
                        "{} {}",
                        dir.display(),
                        aw.id
                    );
                }
            }
        }
        assert!(seen >= 1, "the words file of Moss Hop");
    }
}
