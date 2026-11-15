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
/// its executable or in its data folder.
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
    /// A large-art look's rules file (`gp_present::art`), inside its
    /// folder.
    pub art: Option<String>,
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
        if let Some(a) = &self.art {
            s.push_str(&format!("art {a}\n"));
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
                "art" => {
                    if v.is_empty() || v.starts_with('/') || v.split('/').any(|p| p == "..") {
                        return Err(format!("`{v}` isn't a file inside the look"));
                    }
                    l.art = Some(v.to_owned());
                }
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

/// Where a look's sound for a role is: its first take, the file every page
/// plays (`sound/roles/<role>.wav`).
pub const ROLES_DIR: &str = "sound/roles/";

/// Where a look's further takes of a role are (issue #30):
/// `sound/takes/<role>.<n>.wav`, n from 2. A page from before takes plays
/// only `sound/roles/`, whose every file it takes for a role of its own
/// name, so the takes are kept out of that folder.
pub const TAKES_DIR: &str = "sound/takes/";

/// Whether a role's name is one a model gives (lowercase letters, digits
/// and `_`, gp_model's `SoundRole::name`).
pub fn role_name_ok(role: &str) -> bool {
    !role.is_empty()
        && role
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// The file of take `take` of a role in a look's folder: take 1 is
/// `sound/roles/<role>.wav`, the others `sound/takes/<role>.<take>.wav`.
pub fn take_file(role: &str, take: u32) -> String {
    if take <= 1 {
        format!("{ROLES_DIR}{role}.wav")
    } else {
        format!("{TAKES_DIR}{role}.{take}.wav")
    }
}

/// The role and take a look's file is, by its path: `Some` for
/// `sound/roles/<role>.wav` (take 1) and `sound/takes/<role>.<n>.wav`
/// (n of 2 or more, no leading 0), `None` for any other path.
pub fn role_take(file: &str) -> Option<(&str, u32)> {
    if let Some(role) = file
        .strip_prefix(ROLES_DIR)
        .and_then(|f| f.strip_suffix(".wav"))
    {
        return role_name_ok(role).then_some((role, 1));
    }
    let rest = file.strip_prefix(TAKES_DIR)?.strip_suffix(".wav")?;
    let (role, n) = rest.rsplit_once('.')?;
    if !role_name_ok(role) || n.starts_with('0') || !n.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: u32 = n.parse().ok()?;
    (n >= 2).then_some((role, n))
}

impl Look {
    /// The look's sound files by role: each role's takes (number, file) in
    /// number order, from its `file` lines.
    pub fn role_sounds(&self) -> std::collections::BTreeMap<String, Vec<(u32, String)>> {
        let mut out: std::collections::BTreeMap<String, Vec<(u32, String)>> = Default::default();
        for f in &self.files {
            if let Some((role, take)) = role_take(f) {
                out.entry(role.to_owned())
                    .or_default()
                    .push((take, f.clone()));
            }
        }
        for t in out.values_mut() {
            t.sort();
            t.dedup_by_key(|x| x.0);
        }
        out
    }
}

/// A theme id as the theme format takes it: lowercase letters, digits,
/// `-` and `_`, starting with a letter (`gp_package::theme_id_ok`).
pub fn valid_id(id: &str) -> bool {
    gp_package::theme_id_ok(id)
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
            title: "Paper".into(),
            original: false,
            files: vec!["frame-0-0.png".into(), "sound/roles/jump.wav".into()],
            own_sound: false,
            art: Some("art.txt".into()),
        };
        assert_eq!(Look::read(&l.write()).unwrap(), l);
        assert!(l.has("sound/roles/jump.wav"));
        assert!(Look::read("gp-look 1\nid x\nfile ../a.png\n").is_err());
        assert!(Look::read("gp-look 1\nid x\nart /a.txt\n").is_err());
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

    #[test]
    fn a_roles_takes_are_named_apart_from_its_first_file() {
        assert_eq!(take_file("jump", 1), "sound/roles/jump.wav");
        assert_eq!(take_file("jump", 3), "sound/takes/jump.3.wav");
        assert_eq!(role_take("sound/roles/jump.wav"), Some(("jump", 1)));
        assert_eq!(
            role_take("sound/takes/jump_over.12.wav"),
            Some(("jump_over", 12))
        );
        for no in [
            "sound/takes/jump.1.wav",
            "sound/takes/jump.02.wav",
            "sound/takes/jump.wav",
            "sound/takes/Jump.2.wav",
            "sound/roles/jump.2.wav",
            "sound/roles/jump.ogg",
            "art/jump.wav",
        ] {
            assert_eq!(role_take(no), None, "{no}");
        }
        let l = Look::read(
            "gp-look 1\nid x\nfile sound/takes/hop.3.wav\nfile sound/roles/hop.wav\nfile sound/takes/hop.2.wav\nfile art/a.png\n",
        )
        .unwrap();
        let r = l.role_sounds();
        assert_eq!(
            r["hop"],
            vec![
                (1, "sound/roles/hop.wav".to_owned()),
                (2, "sound/takes/hop.2.wav".to_owned()),
                (3, "sound/takes/hop.3.wav".to_owned())
            ]
        );
    }
}
