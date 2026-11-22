//! What a new game from a ROM is called, so two people porting one ROM get
//! one folder. The id and title come from the games checkout's known list
//! (known/sources.toml) when it names the ROM's hash, else from the arcade
//! set a romset is, else the id from the file's name
//! (crate::roms_cmd::game_id_of). The namespace is the one `gp game new`
//! gives it (crate::game_new::default_namespace). `gp roms scan`, `gp roms
//! add` and `gp progress --next` propose them for each ROM no game names,
//! and `gp game new` takes the title from here when it is given none.

use std::fs;
use std::path::{Path, PathBuf};

use gp_evidence::registry::Registry;
use serde_json::{Value, json};

use crate::game_new::KNOWN_FILE;
use crate::roms::Rom;

/// What a new game from a ROM is called.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Proposal {
    pub id: String,
    /// The ROM's known title; none when only its file names it, and then
    /// `gp game new` titles the game by its id.
    pub title: Option<String>,
    /// The namespace `gp game new` gives it; none with no games checkout,
    /// or games under more than one namespace (then --namespace is needed).
    pub namespace: Option<String>,
    /// Where the id and title come from, in words.
    pub from: String,
}

/// A ROM in the ROM folder no game names, and what its game is called.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NewGame {
    /// The file's name.
    pub rom: String,
    /// Its path in the ROM folder, as `gp roms scan` prints it and `gp game
    /// new --rom` takes it.
    pub rel: PathBuf,
    pub proposal: Proposal,
}

impl Proposal {
    /// The `gp game new` line that makes it, the genre left as GENRE.
    pub(crate) fn line(&self, rel: &Path) -> String {
        crate::roms_cmd::game_new_line(rel, self.title.as_deref().map(|t| (self.id.as_str(), t)))
    }

    /// The title the game gets.
    pub(crate) fn title(&self) -> &str {
        self.title.as_deref().unwrap_or(&self.id)
    }

    pub(crate) fn json(&self) -> Value {
        json!({
            "id": self.id, "title": self.title(), "namespace": self.namespace,
            "from": self.from,
        })
    }
}

/// A game id from a known id: lower-case letters and digits, one dash
/// between runs of them, at most 64 characters.
fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.truncate(64);
    let out = out.trim_end_matches('-').to_owned();
    if out.is_empty() {
        "my-game".into()
    } else {
        out
    }
}

/// What the game from the ROM at `rel` (its path in the ROM folder) is
/// called, by the games checkout `games` (already found): `sha256` is the
/// ROM's identity (an NES image's headerless hash, a romset's), `system` a
/// romset's MAME set.
pub(crate) fn propose(
    games: Option<&Path>,
    rel: &Path,
    sha256: &str,
    system: Option<&str>,
) -> Proposal {
    let known = games
        .and_then(|g| fs::read_to_string(g.join(KNOWN_FILE)).ok())
        .and_then(|t| Registry::from_toml(&t, Path::new(KNOWN_FILE)).ok());
    let namespace = games.and_then(|g| crate::game_new::default_namespace(g).ok());
    if let Some(k) = known.as_ref().and_then(|r| r.by_sha256(sha256)) {
        return Proposal {
            id: slug(&k.id),
            title: Some(k.title.clone()),
            namespace,
            from: format!("the games checkout's {KNOWN_FILE} names it {}", k.id),
        };
    }
    if let Some(s) = system.and_then(gp_arcade::romset::set) {
        return Proposal {
            id: slug(s.name),
            title: Some(s.title.to_owned()),
            namespace,
            from: format!("it is the arcade set {}", s.name),
        };
    }
    Proposal {
        id: crate::roms_cmd::game_id_of(rel),
        title: None,
        namespace,
        from: "no list names it, so its file's name".into(),
    }
}

/// The new games of the ROMs at `paths` in the ROM folder `roms` that no
/// game's roms.toml names, at most `max`, by the games checkout `given`
/// (or the one found).
pub(crate) fn new_games(
    given: Option<&Path>,
    roms: &Path,
    paths: &[PathBuf],
    max: usize,
) -> Vec<NewGame> {
    let games = crate::roms_cmd::games_dir(given);
    let reg = crate::roms_cmd::sources(games.as_deref())
        .map(|(r, _)| r)
        .unwrap_or_default();
    paths
        .iter()
        .filter_map(|p| Some((p, Rom::read(p).ok()?)))
        .filter(|(_, rom)| reg.by_sha256(&rom.sha256).is_none())
        .take(max)
        .map(|(p, rom)| {
            let rel = p.strip_prefix(roms).unwrap_or(p).to_path_buf();
            NewGame {
                rom: p
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                proposal: propose(games.as_deref(), &rel, &rom.sha256, rom.system.as_deref()),
                rel,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_known_id_is_a_game_id() {
        assert_eq!(slug("Rooftop_Run.USA"), "rooftop-run-usa");
        assert_eq!(slug("--Hill*Top--"), "hill-top");
        assert_eq!(slug("***"), "my-game");
        assert_eq!(slug(&format!("{}-b", "a".repeat(63))), "a".repeat(63));
    }

    /// A ROM the known list names gets its id and title; one it doesn't,
    /// the id from its file's name and no title. Two calls on one ROM give
    /// one answer, in the namespace game new gives a checkout's first game.
    #[test]
    fn the_known_list_names_a_rom() {
        let d = tempfile::tempdir().unwrap();
        let sha = "ab".repeat(32);
        let rel = Path::new("nes/Hill Top (USA).nes");
        let p = propose(Some(d.path()), rel, &sha, None);
        assert_eq!((p.id.as_str(), p.title.as_deref()), ("hill-top-usa", None));
        let first = crate::game_new::FIRST_NAMESPACE;
        assert_eq!(p.namespace.as_deref(), Some(first));
        assert_eq!(
            p.line(rel),
            "gp game new hill-top-usa --rom 'nes/Hill Top (USA).nes' --genre GENRE"
        );
        assert_eq!(p.json()["title"], "hill-top-usa");
        fs::create_dir_all(d.path().join("known")).unwrap();
        fs::write(
            d.path().join(KNOWN_FILE),
            format!("[[source]]\nid = \"hilltop-usa\"\ntitle = \"Hilltop\"\nplatform = \"nes\"\nregion = \"ntsc\"\nsha256 = \"{sha}\"\nstatus = \"unverified\"\n"),
        )
        .unwrap();
        let p = propose(Some(d.path()), rel, &sha, None);
        assert_eq!(
            (p.id.as_str(), p.title.as_deref()),
            ("hilltop-usa", Some("Hilltop"))
        );
        assert_eq!(p, propose(Some(d.path()), rel, &sha, None));
        assert_eq!(
            p.line(Path::new("nes/h.nes")),
            "gp game new hilltop-usa --rom nes/h.nes --genre GENRE --title Hilltop"
        );
        assert_eq!(propose(None, rel, &sha, None).namespace, None);
    }
}
