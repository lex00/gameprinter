//! The sound part of `gp theme look check` (issue #30): a large-art look's
//! sound files as the page plays them. A role's file is
//! `sound/roles/<role>.wav` and its further takes `sound/takes/<role>.<n>.wav`
//! (n from 2), each listed in look.txt as a `file` line
//! (`gp_present::looks::role_take`); each start of the role plays its next
//! take in turn (`gp_present::sound::RolePlayer`).
//!
//! What a page from before takes does with them decides what is a problem
//! here: it plays every listed `sound/roles/*.wav` as the role of its file's
//! name and stops loading the look's sounds at one the game hasn't got, and
//! it never reads `sound/takes/`, so it plays a role's first take every
//! time, and none of a role whose takes have no first file.

use std::collections::BTreeMap;
use std::fs;

use gp_present::looks::{Look, ROLES_DIR, TAKES_DIR, role_take, take_file};

use super::{Folder, Game, Group, Level, Report, line_of};

/// Checks the look's sound files against the game's sound roles (its kit's
/// `[[sound]]`), and lists each role's files.
pub(super) fn check(folder: &Folder, look: &Look, on_disk: &[String], game: &Game, r: &mut Report) {
    let look_path = folder.theme.join(gp_present::looks::LOOK_FILE);
    let look_rel = folder.rel(&look_path);
    let text = fs::read_to_string(&look_path).unwrap_or_default();
    let roles = game.kit.as_ref().map(|k| &k.sound);
    let kit_role = |name: &str| roles.and_then(|rs| rs.iter().find(|s| s.role == name));
    let listed: Vec<&String> = look
        .files
        .iter()
        .filter(|f| f.starts_with("sound/"))
        .collect();

    if look.own_sound && !listed.is_empty() {
        r.add(
            Group::Sound,
            Level::Warning,
            Some(&look_rel),
            line_of(&text, "own_sound"),
            "says `own_sound`, so the page plays the game's own sound and none of the files below. Take the line out to play them",
        );
    }

    // Each listed file: a place the page reads, a role the game has, a
    // sound the page can play.
    let mut good: BTreeMap<String, Vec<(u32, String)>> = BTreeMap::new();
    for f in &listed {
        let line = line_of(&text, f.as_str());
        let Some((role, take)) = role_take(f) else {
            if f.starts_with(ROLES_DIR) {
                r.add(
                    Group::Sound,
                    Level::Problem,
                    Some(&look_rel),
                    line,
                    format!("names {f}, which isn't a role's file: in {ROLES_DIR} a page plays each file as the role of its name, and stops loading the theme's sounds at a role the game hasn't got. A role's file is {ROLES_DIR}ROLE.wav; its other takes go in {TAKES_DIR} as ROLE.2.wav, ROLE.3.wav and on"),
                );
            } else {
                r.add(
                    Group::Sound,
                    Level::Warning,
                    Some(&look_rel),
                    line,
                    format!("names {f}, which the page never plays: a role's file is {ROLES_DIR}ROLE.wav and its other takes {TAKES_DIR}ROLE.2.wav, ROLE.3.wav and on"),
                );
            }
            continue;
        };
        if roles.is_some() && kit_role(role).is_none() {
            let names: Vec<&str> = roles
                .into_iter()
                .flatten()
                .map(|s| s.role.as_str())
                .collect();
            r.add(
                Group::Sound,
                Level::Problem,
                Some(&look_rel),
                line,
                format!(
                    "names {f}, but {} has no sound role `{role}`{}. {}",
                    game.title,
                    if names.is_empty() {
                        " (it has none)".to_owned()
                    } else {
                        format!(" (its roles: {})", names.join(", "))
                    },
                    if take == 1 {
                        "A page stops loading the theme's sounds at it, so the roles after it play nothing"
                    } else {
                        "The page passes over it"
                    }
                ),
            );
            continue;
        }
        let p = folder.theme.join(f.as_str());
        let Ok(bytes) = fs::read(&p) else {
            r.add(
                Group::Sound,
                Level::Problem,
                Some(&look_rel),
                line,
                format!(
                    "names {f}, which isn't in the theme folder. Add {} or take the line out",
                    folder.theme_rel(f)
                ),
            );
            continue;
        };
        match gp_present::sound::read_wav(&bytes) {
            Err(e) => r.add(
                Group::Sound,
                Level::Problem,
                Some(&folder.theme_rel(f)),
                None,
                format!("isn't a sound the page plays ({e}). Save it as a 16-bit PCM WAV"),
            ),
            Ok(s) if s.is_empty() => r.add(
                Group::Sound,
                Level::Problem,
                Some(&folder.theme_rel(f)),
                None,
                "is a WAV with no sound in it",
            ),
            Ok(_) => good
                .entry(role.to_owned())
                .or_default()
                .push((take, (*f).clone())),
        }
    }

    // Each role's files, and what a page makes of them.
    for (role, takes) in &mut good {
        takes.sort();
        takes.dedup_by_key(|t| t.0);
        let files: Vec<&str> = takes.iter().map(|t| t.1.as_str()).collect();
        if takes[0].0 != 1 {
            r.add(
                Group::Sound,
                Level::Problem,
                Some(&look_rel),
                line_of(&text, &takes[0].1),
                format!("`{role}` has takes but no {}: a page from before takes plays only that file, so it plays none of them. Make one of them {}", take_file(role, 1), take_file(role, 1)),
            );
            continue;
        }
        let kit = kit_role(role);
        let effect = kit.is_some_and(|k| k.kind == "sfx" && !k.loops);
        if takes.len() == 1 {
            r.add(
                Group::Sound,
                Level::Ok,
                None,
                None,
                format!(
                    "`{role}`: 1 file ({}), played every time it starts",
                    files[0]
                ),
            );
            if effect {
                r.add(
                    Group::Sound,
                    Level::Warning,
                    Some(&folder.theme_rel(files[0])),
                    None,
                    format!("`{role}` has one file, so this effect sounds the same every time it plays. One that fires often (a jump, a step, a hit) sounds better with two or three takes: add {} and {} and list them in look.txt", take_file(role, 2), take_file(role, 3)),
                );
            }
        } else {
            let numbers: Vec<u32> = takes.iter().map(|t| t.0).collect();
            let gaps = numbers.iter().enumerate().any(|(i, n)| *n != i as u32 + 1);
            r.add(
                Group::Sound,
                Level::Ok,
                None,
                None,
                format!(
                    "`{role}`: {} files ({}), each start playing the next in turn; a page from before takes plays the first every time",
                    takes.len(),
                    files.join(", ")
                ),
            );
            if gaps {
                r.add(
                    Group::Sound,
                    Level::Note,
                    Some(&look_rel),
                    None,
                    format!("`{role}`'s takes are numbered {}: they play in that order, the gaps passed over", numbers.iter().map(u32::to_string).collect::<Vec<_>>().join(", ")),
                );
            }
        }
    }

    // Files in the folder that no line lists.
    let unlisted: Vec<String> = on_disk
        .iter()
        .filter(|f| (f.starts_with(ROLES_DIR) || f.starts_with(TAKES_DIR)) && !look.has(f))
        .map(|f| folder.theme_rel(f))
        .collect();
    if !unlisted.is_empty() {
        r.add(
            Group::Sound,
            Level::Note,
            Some(&look_rel),
            None,
            format!(
                "not listed in look.txt, so the page never plays {}: {}. List each as a `file` line",
                if unlisted.len() == 1 { "it" } else { "them" },
                unlisted.join(", ")
            ),
        );
    }

    // The roles the theme leaves without a sound.
    let Some(roles) = roles.filter(|rs| !rs.is_empty()) else {
        return;
    };
    if good.is_empty() {
        r.add(
            Group::Sound,
            Level::Note,
            None,
            None,
            format!(
                "the theme gives no sound role a file, so the game keeps its own sound ({} sound roles in the kit). A role's sound is {ROLES_DIR}ROLE.wav, listed in look.txt as `file {ROLES_DIR}ROLE.wav`",
                roles.len()
            ),
        );
    } else if !look.own_sound {
        let silent: Vec<&str> = roles
            .iter()
            .filter(|s| !good.contains_key(&s.role))
            .map(|s| s.role.as_str())
            .collect();
        if !silent.is_empty() {
            r.add(
                Group::Sound,
                Level::Note,
                None,
                None,
                format!(
                    "with any sound of its own the theme silences the game's, so these roles play nothing: {}",
                    silent.join(", ")
                ),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::report;
    use super::super::tests::{GOOD_PACKAGE, game_with_sound, levels, put, theme};

    /// A short made-up tone (a square wave), 16-bit mono: nothing from a
    /// ROM.
    pub(crate) fn tone(period: usize, samples: usize) -> Vec<u8> {
        let s: Vec<i16> = (0..samples)
            .map(|i| {
                if (i / period).is_multiple_of(2) {
                    4000
                } else {
                    -4000
                }
            })
            .collect();
        gp_theme::tune::wav(&s, gp_present::sound::ROLE_RATE)
    }

    /// A made-up game with an effect `hop` and a looping tune `tune`, and
    /// made-up themes: three takes of `hop` listed and each read; one take
    /// warned of for the effect and not for the tune; a take whose role has
    /// no first file, a role the game hasn't got, a WAV the page can't play,
    /// a missing file and a take put in `sound/roles/` each a problem; an
    /// unlisted file noted.
    #[test]
    fn the_check_lists_each_roles_takes_and_refuses_what_a_page_cant_play() {
        let g = tempfile::tempdir().unwrap();
        let game = game_with_sound(g.path());
        let t = tempfile::tempdir().unwrap();
        let art = "gp-art 1\nscale 4\npic frame-0-0.png art/roll.png 16 16 0 0\npic frame-1-0.png art/roll.png 16 16 0 0\n";
        let run = |look: &str| {
            theme(t.path(), GOOD_PACKAGE, look, art);
            put(
                t.path(),
                "theme/art/roll.png",
                &super::super::tests::picture(64, 64, [8, 8, 56, 64]).png(),
            );
            report(t.path(), &game, Ok(super::super::tests::built()), None).unwrap()
        };
        for (f, p) in [
            ("hop.wav", 20),
            ("tune.wav", 50),
            ("hop.2.wav", 30),
            ("hop.3.wav", 40),
        ] {
            let dir = if f.matches('.').count() == 2 {
                "takes"
            } else {
                "roles"
            };
            put(t.path(), &format!("theme/sound/{dir}/{f}"), &tone(p, 800));
        }
        let base = "gp-look 1\nid bright\ntitle Bright\nart art.txt\n";
        let three = format!(
            "{base}file sound/roles/hop.wav\nfile sound/takes/hop.2.wav\nfile sound/takes/hop.3.wav\nfile sound/roles/tune.wav\n"
        );
        let o = run(&three);
        assert_eq!(o.exit, 0, "{}", o.text);
        assert!(
            o.text.contains("`hop`: 3 files (sound/roles/hop.wav, sound/takes/hop.2.wav, sound/takes/hop.3.wav), each start playing the next in turn"),
            "{}",
            o.text
        );
        assert!(
            o.text.contains("`tune`: 1 file (sound/roles/tune.wav)"),
            "{}",
            o.text
        );
        assert!(levels(&o, "warning").is_empty(), "{}", o.text);

        // One take of the effect: a warning; the looping tune: none.
        let one = format!("{base}file sound/roles/hop.wav\nfile sound/roles/tune.wav\n");
        let o = run(&one);
        let w = levels(&o, "warning");
        assert_eq!(w.len(), 1, "{}", o.text);
        assert!(
            w[0].contains("`hop` has one file, so this effect sounds the same every time it plays"),
            "{w:?}"
        );
        assert!(o.text.contains("not listed in look.txt, so the page never plays them: theme/sound/takes/hop.2.wav, theme/sound/takes/hop.3.wav"), "{}", o.text);

        // The mistakes, each a problem by its line.
        put(
            t.path(),
            "theme/sound/roles/hiss.wav",
            b"RIFF\0\0\0\0WAVEnope",
        );
        put(t.path(), "theme/sound/takes/tune.2.wav", &tone(60, 800));
        let bad = format!(
            "{base}file sound/takes/hop.2.wav\nfile sound/roles/boom.wav\nfile sound/roles/tune.wav\nfile sound/roles/hop.4.wav\nfile sound/roles/tune.wav\nfile sound/takes/tune.2.wav\nfile sound/takes/tune.3.wav\n"
        );
        let o = run(&bad);
        let p = levels(&o, "problem");
        let has = |s: &str| p.iter().any(|x| x.contains(s));
        assert!(
            has(
                "5 `hop` has takes but no sound/roles/hop.wav: a page from before takes plays only that file, so it plays none of them"
            ),
            "{p:?}"
        );
        assert!(
            has(
                "6 names sound/roles/boom.wav, but Made Up has no sound role `boom` (its roles: hop, tune). A page stops loading"
            ),
            "{p:?}"
        );
        assert!(
            has("8 names sound/roles/hop.4.wav, which isn't a role's file"),
            "{p:?}"
        );
        assert!(
            has("11 names sound/takes/tune.3.wav, which isn't in the theme folder"),
            "{p:?}"
        );
        assert_eq!(o.exit, 1);
        let unplayable = format!("{base}file sound/roles/hop.wav\nfile sound/roles/tune.wav\n");
        put(
            t.path(),
            "theme/sound/roles/hop.wav",
            b"RIFF\0\0\0\0WAVEfmt ",
        );
        let o = run(&unplayable);
        let p = levels(&o, "problem");
        assert!(
            p.iter().any(|x| x.contains("theme/sound/roles/hop.wav")
                && x.contains("isn't a sound the page plays")),
            "{p:?}"
        );
    }
}
