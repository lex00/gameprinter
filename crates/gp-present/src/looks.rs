//! Themes a runtime finds when it runs (docs/formats/theme.md, "Themes
//! found at run time"). A look is a theme drawn for one game: a folder
//! `themes/<id>/` holding `look.txt` and the files of the game's own
//! `assets/` that the theme changes, under the same names (its pictures,
//! `present.txt`, the overlays and `sound/roles/<role>.wav`). A runtime
//! shows a picture from the look's folder when the look has it, else the
//! build's own. The build's own look is the one its theme stack drew; the
//! look `original` is the lifted art and the game's own sound, which a
//! build of a game lifted from a ROM writes when its stack draws a theme.
//!
//! Looks are presentation: nothing in the simulation reads them, so a
//! runtime changes look between two frames with no restart.

/// The manifest's file name in a look's folder.
pub const LOOK_FILE: &str = "look.txt";

/// The manifest's first line.
pub const LOOK_HEADER: &str = "gp-look 1";

/// The folder of looks beside a build's bundle (the web preview), beside
/// its executable or in its data folder (the Bevy build).
pub const THEMES_DIR: &str = "themes";

/// The web build's list of the looks in its `themes/` folder, which a
/// static host serves as it is: `{"themes": [{"id": ..., "title": ...}]}`.
pub const INDEX_FILE: &str = "index.json";

/// The look of the lifted art and the game's own sound.
pub const ORIGINAL: &str = "original";

/// A look's manifest.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Look {
    pub id: String,
    pub title: String,
    /// Whether it is the lifted art (`original`): shown only where the
    /// ROM's assets were lifted.
    pub original: bool,
    /// The files it holds, relative to its folder, as under `assets/`.
    pub files: Vec<String>,
    /// Whether the game's own sound plays (no sound role replaced). A look
    /// with any `sound/roles/` file replaces the game's sound.
    pub own_sound: bool,
}

impl Look {
    pub fn write(&self) -> String {
        let mut s = format!("{LOOK_HEADER}\nid {}\ntitle {}\n", self.id, self.title);
        if self.original {
            s.push_str("original\n");
        }
        if self.own_sound {
            s.push_str("own_sound\n");
        }
        for f in &self.files {
            s.push_str(&format!("file {f}\n"));
        }
        s
    }

    pub fn read(text: &str) -> Result<Self, String> {
        let mut lines = text.lines();
        if lines.next().map(str::trim) != Some(LOOK_HEADER) {
            return Err(format!("not a look (no `{LOOK_HEADER}` line)"));
        }
        let mut l = Look::default();
        for line in lines {
            let line = line.trim();
            let (k, v) = line.split_once(' ').unwrap_or((line, ""));
            match k {
                "" => {}
                "id" => l.id = v.to_owned(),
                "title" => l.title = v.to_owned(),
                "original" => l.original = true,
                "own_sound" => l.own_sound = true,
                "file" => {
                    if v.is_empty() || v.starts_with('/') || v.split('/').any(|p| p == "..") {
                        return Err(format!("`{v}` isn't a file inside the look"));
                    }
                    l.files.push(v.to_owned());
                }
                _ => return Err(format!("`{line}`")),
            }
        }
        if !valid_id(&l.id) {
            return Err(format!("`{}` isn't a theme id", l.id));
        }
        if l.title.is_empty() {
            l.title = l.id.clone();
        }
        Ok(l)
    }

    /// Whether it holds a file.
    pub fn has(&self, file: &str) -> bool {
        self.files.iter().any(|f| f == file)
    }
}

/// A theme id as the theme format takes it: lowercase letters, digits,
/// `-` and `_`, starting with a letter.
pub fn valid_id(id: &str) -> bool {
    id.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// The index a static host serves (`themes/index.json`): each look's id
/// and title.
pub fn write_index(looks: &[(String, String)]) -> String {
    let items: Vec<String> = looks
        .iter()
        .map(|(id, title)| format!("{{\"id\": {}, \"title\": {}}}", json(id), json(title)))
        .collect();
    format!("{{\"themes\": [{}]}}\n", items.join(", "))
}

fn json(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The looks a settings screen lists: the build's own first, then each
/// one found, once by id (the first found wins), `original` only where
/// `lifted`.
pub fn choices(own: (&str, &str), found: &[Look], lifted: bool) -> Vec<(String, String)> {
    let mut out = vec![(own.0.to_owned(), own.1.to_owned())];
    for l in found {
        if (l.original || l.id == ORIGINAL) && !lifted {
            continue;
        }
        if out.iter().any(|(id, _)| *id == l.id) {
            continue;
        }
        out.push((l.id.clone(), l.title.clone()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_look_round_trips_and_refuses_paths_out_of_its_folder() {
        let l = Look {
            id: "paper".into(),
            title: "Rivet Rodeo".into(),
            original: false,
            files: vec!["frame-0-0.png".into(), "sound/roles/jump.wav".into()],
            own_sound: false,
        };
        assert_eq!(Look::read(&l.write()).unwrap(), l);
        assert!(l.has("sound/roles/jump.wav"));
        assert!(Look::read("gp-look 1\nid x\nfile ../a.png\n").is_err());
        assert!(Look::read("gp-look 1\nid X\n").is_err());
        assert!(Look::read("nope").is_err());
        assert_eq!(
            write_index(&[("paper".into(), "Paper \"P\"".into())]),
            "{\"themes\": [{\"id\": \"paper\", \"title\": \"Paper \\\"P\\\"\"}]}\n"
        );
        let orig = Look {
            id: ORIGINAL.into(),
            title: "Original".into(),
            original: true,
            ..Look::default()
        };
        let found = [l.clone(), orig.clone(), l.clone()];
        assert_eq!(choices(("own", "Own"), &found, false).len(), 2);
        assert_eq!(choices(("own", "Own"), &found, true).len(), 3);
    }
}
