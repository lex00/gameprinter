//! Style cards (docs/theme-kit.md, "Style cards"): a small file that says
//! how a theme looks, apart from any game, so one card makes a theme for
//! Donkey Kong, Moss Hop or a new game alike. `gp theme sheet --style
//! NAME` merges it into every prompt it writes.
//!
//! Shareable cards are `themes/styles/<name>.toml` in the repo and name no
//! game; personal ones are `$GP_HOME/themes/styles/<name>.toml`. A name is
//! looked for in the personal folder first.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use gp_evidence::Paths;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::Outcome;

/// The card format's version.
pub const CARD_FORMAT: &str = "0.1.0";

/// A style card.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Card {
    pub format: String,
    /// Lowercase letters, digits and `-`; the file's name.
    pub name: String,
    pub title: String,
    pub author: String,
    /// The card's own license, an SPDX expression (default CC0-1.0).
    #[serde(default = "cc0")]
    pub license: String,
    /// The look in a few lines: lines, colors, period, proportions.
    pub look: String,
    /// The cast, in the person's own words: who the hero is, what the foes
    /// are. It names no game and no character of one.
    #[serde(default)]
    pub cast: String,
    /// Palette hints, `#rrggbb`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub palette: Vec<String>,
    /// The style board, once made: a picture path, relative to the card's
    /// folder or absolute.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board: Option<String>,
    /// Turnarounds, once made: a picture per subject, by the subject's
    /// label (`hero`, `big foe`, `the one to rescue`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub turnaround: Vec<Turnaround>,
}

fn cc0() -> String {
    "CC0-1.0".into()
}

/// A turnaround picture of one subject.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Turnaround {
    /// The subject's label, as the kit's slots have them.
    pub subject: String,
    pub image: String,
}

impl Card {
    /// The card's problems: its name, its colors, its text.
    pub fn check(&self) -> Vec<String> {
        let mut out = Vec::new();
        if !valid_name(&self.name) {
            out.push(format!(
                "name `{}`: lowercase letters, digits and -, starting with a letter",
                self.name
            ));
        }
        if self.look.trim().is_empty() {
            out.push("look is empty".into());
        }
        for c in &self.palette {
            if gp_theme::color::Rgb::parse(c).is_err() {
                out.push(format!("palette `{c}` isn't #rrggbb"));
            }
        }
        for (k, v) in [
            ("look", &self.look),
            ("cast", &self.cast),
            ("title", &self.title),
        ] {
            if !v.is_ascii() {
                out.push(format!("{k} isn't plain ASCII"));
            }
        }
        out
    }

    /// A path the card names, resolved against its folder.
    pub fn resolve(&self, dir: &Path, p: &str) -> PathBuf {
        let p = Path::new(p);
        if p.is_absolute() {
            p.to_owned()
        } else {
            dir.join(p)
        }
    }

    /// The card's text, with a header for people.
    pub fn text(&self) -> String {
        format!(
            "# A style card (docs/theme-kit.md, \"Style cards\"): how a theme looks,\n# apart from any game. `gp theme sheet GAME --style {}` puts it into\n# every prompt.\n{}",
            self.name,
            toml::to_string(self).expect("a card serializes")
        )
    }
}

fn valid_name(n: &str) -> bool {
    n.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && n.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// The repo's shareable cards: gameprinter's `themes/styles/` (game-printer
/// read them from the working folder, beside `templates/`).
pub fn shared_dir() -> PathBuf {
    crate::serve::repo_root().join("themes").join("styles")
}

/// The personal cards: `$GP_HOME/themes/styles/`.
pub fn personal_dir(paths: &Paths) -> PathBuf {
    paths.home.join("themes").join("styles")
}

/// A card by name (personal first, then shared) or by file path, with
/// its file.
pub fn find(arg: &str, paths: &Paths) -> Result<(Card, PathBuf)> {
    let p = Path::new(arg);
    let file = if arg.ends_with(".toml") && p.is_file() {
        p.to_owned()
    } else {
        let mine = personal_dir(paths).join(format!("{arg}.toml"));
        let shared = shared_dir().join(format!("{arg}.toml"));
        if mine.is_file() {
            mine
        } else if shared.is_file() {
            shared
        } else {
            bail!(
                "no style card `{arg}` in {} or {}; make one with `gp theme style new`",
                personal_dir(paths).display(),
                shared_dir().display()
            )
        }
    };
    let card = load(&file)?;
    Ok((card, file))
}

/// A card file.
pub fn load(file: &Path) -> Result<Card> {
    let text = fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    let card: Card = toml::from_str(&text).with_context(|| format!("{}", file.display()))?;
    let p = card.check();
    if !p.is_empty() {
        bail!("{}: {}", file.display(), p.join("; "));
    }
    Ok(card)
}

#[derive(Subcommand)]
pub enum StyleCmd {
    /// Write a style card with the look, the cast in your own words and palette
    /// hints. Shareable cards go to themes/styles/ in the repo and must name no
    /// game; --personal writes $GP_HOME/themes/styles/.
    New {
        /// The card's name: lowercase letters, digits and -.
        name: String,
        /// The look in a few lines (lines, colors, period, proportions).
        #[arg(long)]
        look: String,
        /// The cast in your own words: the hero, the foes, the world.
        #[arg(long, default_value = "")]
        cast: String,
        /// A palette hint, #rrggbb; repeat for more.
        #[arg(long = "palette", value_name = "COLOR")]
        palette: Vec<String>,
        #[arg(long, default_value = "")]
        title: String,
        #[arg(long, default_value = "")]
        author: String,
        /// A style board picture already made.
        #[arg(long, value_name = "FILE")]
        board: Option<String>,
        /// Write to $GP_HOME/themes/styles/ instead of the repo.
        #[arg(long)]
        personal: bool,
        #[arg(long)]
        force: bool,
    },
    /// List the style cards, personal and shared.
    List,
    /// Print a style card.
    Show { name: String },
}

/// The words no shareable card may hold: every template's denied words
/// and each model's title words.
pub fn game_words() -> Vec<String> {
    let mut out = Vec::new();
    for d in crate::theme_template::game_dirs() {
        if let Ok(Some(w)) = crate::theme_words::load(&d) {
            out.extend(w.deny);
        }
    }
    out.sort();
    out.dedup();
    out
}

/// A shareable card's problems: a game's word in its text.
pub fn shared_problems(card: &Card, deny: &[String]) -> Vec<String> {
    let text = format!(
        "{} {} {} {}",
        card.name.replace('-', " "),
        card.title,
        card.look,
        card.cast
    );
    let d = crate::theme_words::denied(&text, deny);
    if d.is_empty() {
        Vec::new()
    } else {
        vec![format!(
            "a shareable card names no game, and `{}` holds {}",
            card.name,
            d.join(", ")
        )]
    }
}

pub fn run(cmd: &StyleCmd, paths: &Paths) -> Result<Outcome> {
    match cmd {
        StyleCmd::New {
            name,
            look,
            cast,
            palette,
            title,
            author,
            board,
            personal,
            force,
        } => {
            let card = Card {
                format: CARD_FORMAT.into(),
                name: name.clone(),
                title: if title.is_empty() {
                    name.clone()
                } else {
                    title.clone()
                },
                author: author.clone(),
                license: cc0(),
                look: look.clone(),
                cast: cast.clone(),
                palette: palette.clone(),
                board: board.clone(),
                turnaround: Vec::new(),
            };
            let mut p = card.check();
            if !personal {
                p.extend(shared_problems(&card, &game_words()));
            }
            if !p.is_empty() {
                bail!("{}", p.join("; "));
            }
            let dir = if *personal {
                personal_dir(paths)
            } else {
                shared_dir()
            };
            fs::create_dir_all(&dir)?;
            let file = dir.join(format!("{name}.toml"));
            if file.exists() && !force {
                bail!("{} exists; pass --force to replace it", file.display());
            }
            fs::write(&file, card.text())?;
            Ok(Outcome::ok(
                json!({ "card": file }),
                format!("wrote {}", file.display()),
            ))
        }
        StyleCmd::List => {
            let mut rows = Vec::new();
            for (scope, dir) in [("personal", personal_dir(paths)), ("shared", shared_dir())] {
                let Ok(rd) = fs::read_dir(&dir) else { continue };
                let mut files: Vec<PathBuf> = rd
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|e| e == "toml"))
                    .collect();
                files.sort();
                for f in files {
                    match load(&f) {
                        Ok(c) => rows.push(json!({ "scope": scope, "name": c.name, "title": c.title, "file": f, "board": c.board.is_some(), "turnarounds": c.turnaround.len() })),
                        Err(e) => rows.push(json!({ "scope": scope, "file": f, "error": format!("{e:#}") })),
                    }
                }
            }
            let text = rows
                .iter()
                .map(|r| match r.get("error") {
                    Some(e) => format!(
                        "{:8}  {}  ERROR {e}",
                        r["scope"].as_str().unwrap_or(""),
                        r["file"]
                    ),
                    None => format!(
                        "{:8}  {:20}  {}",
                        r["scope"].as_str().unwrap_or(""),
                        r["name"].as_str().unwrap_or(""),
                        r["title"].as_str().unwrap_or("")
                    ),
                })
                .collect::<Vec<_>>()
                .join("\n");
            Ok(Outcome::ok(
                json!({ "cards": rows }),
                if text.is_empty() {
                    "no style cards".into()
                } else {
                    text
                },
            ))
        }
        StyleCmd::Show { name } => {
            let (card, file) = find(name, paths)?;
            Ok(Outcome::ok(
                json!({ "file": file, "card": card }),
                format!("# {}\n{}", file.display(), card.text()),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every shareable card in the repo loads, checks and names no game.
    #[test]
    fn shared_cards_check_and_name_no_game() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut deny = Vec::new();
        for d in crate::theme_template::test_game_dirs() {
            if let Some(w) = crate::theme_words::load(&d).unwrap() {
                deny.extend(w.deny);
            }
        }
        let mut n = 0;
        for e in fs::read_dir(root.join("themes/styles")).unwrap().flatten() {
            let card = load(&e.path()).unwrap();
            assert!(shared_problems(&card, &deny).is_empty(), "{}", card.name);
            assert_eq!(
                e.path().file_stem().unwrap().to_string_lossy(),
                card.name,
                "a card's file is its name"
            );
            n += 1;
        }
        assert!(n >= 1, "the example card");
        let bad = Card {
            name: "girder-days".into(),
            look: "a poster".into(),
            ..Card::default()
        };
        assert!(!shared_problems(&bad, &["girder".into()]).is_empty());
    }
}
