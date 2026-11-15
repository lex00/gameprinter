//! A visitor's arcade romset lifted in their browser (issue 3): the zip
//! the player picks is read in memory (`gp_arcade::romset::read_zip`, which
//! inflates with flate2's pure-Rust backend and touches no file or network),
//! the set is known by its chips' SHA-1s whatever the files are named
//! (`gp_arcade::Romset::from_zip`), it must be one of the sets the game's
//! templates accept, and its pictures and tables are lifted with the
//! game's arcade asset map (`gp_assets::arcade::lift`), the same code
//! `gp assets lift` runs. Nothing from the zip is written anywhere but the
//! lift's own output folder, which on the page is the wasi shim's memory.

use std::path::Path;

use gp_model::Model;

use crate::{Entry, GameFile, Load, Named, Outcome, Refused, named, read, write};

/// A set's graphics regions decoded and its program, for the arcade lift.
pub fn decoded(set: &gp_arcade::Romset) -> gp_assets::arcade::Decoded {
    let g = gp_arcade::video::Gfx::decode(&set.gfx1, &set.gfx2);
    gp_assets::arcade::Decoded {
        set: set.name.to_owned(),
        identity: set.identity.clone(),
        tiles: g.tiles,
        sprites: g.sprites,
        regions: [("maincpu".to_owned(), set.maincpu.clone())].into(),
    }
}

/// Lifts `rom` (an NES image's or a romset zip's bytes) with the map's
/// text, whichever platform the map is for: the files and what each asset
/// gave. `gp assets lift` and the page's lift both call this.
pub fn lift_files(
    rom: &[u8],
    map_text: &str,
) -> Result<(Vec<gp_assets::File>, Vec<gp_assets::Lifted>), String> {
    if gp_assets::arcade::is_arcade(map_text) {
        let map = gp_assets::arcade::ArcadeMap::parse(map_text).map_err(|e| e.to_string())?;
        if !gp_arcade::romset::looks_like_zip(rom) {
            return Err("an arcade asset map lifts a romset zip, and this ROM isn't one".into());
        }
        let set = gp_arcade::Romset::from_zip(rom).map_err(|e| e.to_string())?;
        return gp_assets::arcade::lift(&decoded(&set), &map, map_text).map_err(|e| e.to_string());
    }
    let map = gp_assets::AssetMap::parse(map_text).map_err(|e| e.to_string())?;
    gp_assets::lift(rom, &map, map_text).map_err(|e| e.to_string())
}

/// Whether a game folder's templates are arcade ones: the first
/// template's asset map is an arcade map.
pub(crate) fn is_arcade_game(dir: &Path, game: &GameFile) -> bool {
    game.templates.first().is_some_and(|id| {
        std::fs::read_to_string(dir.join(id).join("asset-map.toml"))
            .is_ok_and(|t| gp_assets::arcade::is_arcade(&t))
    })
}

/// A template folder of an arcade game and the set identities it takes:
/// a template's own `source.accepts`, or for a folder of a set loaded only
/// to play (its `load.json`) the `accepts` of its template's `[[loads]]`
/// entry of the same source.
struct Taker {
    id: String,
    accepts: Vec<String>,
}

fn strings(v: Option<&toml::Value>) -> Vec<String> {
    v.and_then(toml::Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn takers(dir: &Path, game: &GameFile) -> Result<Vec<Taker>, String> {
    let mut out = Vec::new();
    for id in &game.templates {
        let t = dir.join(id);
        let manifest: toml::Table = toml::from_str(&read(&t.join("template.toml"))?)
            .map_err(|e| format!("{id}: template.toml: {e}"))?;
        let accepts = if t.join(crate::LOAD_FILE).is_file() {
            manifest
                .get("loads")
                .and_then(toml::Value::as_array)
                .into_iter()
                .flatten()
                .find(|l| l.get("source").and_then(toml::Value::as_str) == Some(id.as_str()))
                .map(|l| strings(l.get("accepts")))
                .unwrap_or_default()
        } else {
            strings(manifest.get("source").and_then(|s| s.get("accepts")))
        };
        out.push(Taker {
            id: id.clone(),
            accepts,
        });
    }
    Ok(out)
}

/// A set by its identity: the registry's name for it, else the board's
/// own list's title.
fn set_named(registry: &[Entry], identity: &str) -> Named {
    let mut n = named(registry, identity);
    // An arcade set's revision is its whole title ("Tapper (Root Beer)",
    // MAME's), so it is the name as it stands, not "Tapper (Tapper (Root
    // Beer))".
    if let Some(r) = registry
        .iter()
        .find(|e| {
            e.sha256
                .as_deref()
                .is_some_and(|h| h.eq_ignore_ascii_case(identity))
        })
        .and_then(|e| Some((e.title.as_deref()?, e.revision.as_deref()?)))
        .filter(|(t, r)| r.starts_with(t))
        .map(|(_, r)| r.to_owned())
    {
        n.name = Some(r);
    }
    if n.name.is_none()
        && let Some(s) = gp_arcade::romset::SETS
            .iter()
            .find(|s| s.identity().eq_ignore_ascii_case(identity))
    {
        n.id = Some(s.name.to_owned());
        n.title = Some(s.title.to_owned());
        n.name = Some(s.title.to_owned());
    }
    n
}

/// MAME's name of the set with this identity, when the board knows it.
fn mame_name(identity: &str) -> Option<&'static str> {
    gp_arcade::romset::SETS
        .iter()
        .find(|s| s.identity().eq_ignore_ascii_case(identity))
        .map(|s| s.name)
}

/// The sets a game takes, for a sentence: "tapper, rbtapper, ... or
/// tappera", by MAME's names.
fn sets_sentence(expected: &[Named]) -> String {
    let names: Vec<String> = expected
        .iter()
        .map(|n| {
            mame_name(&n.sha256)
                .map(str::to_owned)
                .or_else(|| n.id.clone())
                .unwrap_or_else(|| n.sha256.clone())
        })
        .collect();
    match names.as_slice() {
        [] => "none".into(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} or {last}", rest.join(", ")),
    }
}

/// Names for a sentence: "a, b and c".
fn sets_sentence_names(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [one] => (*one).to_owned(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// The chips of the nearest set a zip lacks, by name: what makes it
/// incomplete.
fn missing_chips(rom: &[u8], set: &str) -> Vec<&'static str> {
    let Some(info) = gp_arcade::romset::set(set) else {
        return Vec::new();
    };
    let Ok(files) = gp_arcade::romset::read_zip(rom) else {
        return Vec::new();
    };
    let have: std::collections::HashSet<String> = files
        .iter()
        .map(|(_, d)| {
            gp_arcade::sha1::sha1(d)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect()
        })
        .collect();
    info.chips
        .iter()
        .filter(|c| !have.contains(c.sha1))
        .map(|c| c.name)
        .collect()
}

/// Whether two sets run the same program: the same main CPU chips. Unknown
/// sets don't.
fn same_program(a: &str, b: &str) -> bool {
    let program = |name: &str| {
        gp_arcade::romset::set(name).map(|s| {
            let mut v: Vec<&str> = s
                .chips
                .iter()
                .filter(|c| c.region == gp_arcade::romset::Region::MainCpu)
                .map(|c| c.sha1)
                .collect();
            v.sort_unstable();
            v
        })
    };
    matches!((program(a), program(b)), (Some(x), Some(y)) if x == y)
}

/// The program tables the map reads at a place it never checked for
/// `set`, by name: when the set runs another program than the map's own
/// set, every table the map reads from the program (`maincpu`) needs a
/// place recorded for it (`[asset.at]`, the table's `offset` again where
/// the set keeps it at the same place). A table with none is read at the
/// map's own set's place, which in that program may hold other bytes, with
/// no error: the game is drawn with a garbled screen, wrong colours or a
/// wrong pose (docs/web.md, "A set's tables"). A set with any is refused,
/// by name and naming them, until their places are recorded. Empty for the
/// map's own set and a set with the same program chips.
pub(crate) fn tables_unplaced(map: &gp_assets::arcade::ArcadeMap, set: &str) -> Vec<String> {
    if set == map.source || same_program(set, &map.source) {
        return Vec::new();
    }
    map.assets
        .iter()
        .filter(|a| a.format.table() && a.region == "maincpu" && !a.at.contains_key(set))
        .map(|a| a.name.clone())
        .collect()
}

/// Tables named for a sentence: the first few, and how many more.
fn tables_words(names: &[String]) -> String {
    const SHOWN: usize = 3;
    let shown: Vec<&str> = names.iter().take(SHOWN).map(String::as_str).collect();
    match names.len() {
        n if n <= SHOWN => sets_sentence_names(&shown),
        n => format!("{} and {} more", shown.join(", "), n - SHOWN),
    }
}

/// A refusal in words a player reads, naming the game and the sets it
/// takes.
fn refuse(error: String, expected: Vec<Named>, got: Named) -> Outcome {
    Outcome::Refused(Refused {
        error,
        expected,
        got,
    })
}

/// [`crate::lift_game`] for an arcade game: the player's zip must be a
/// whole romset of one of the sets its templates take. The set's folder
/// gives the model (a set loaded only to play takes its template's, with
/// its load applied), the arcade map lifts it into `out/lifted/`, and the
/// preview's files are written from that as for any game.
pub(crate) fn lift_game(
    rom: &[u8],
    dir: &Path,
    out: &Path,
    game: &GameFile,
    registry: &[Entry],
) -> Result<Outcome, String> {
    let takers = takers(dir, game)?;
    let expected: Vec<Named> = takers
        .iter()
        .flat_map(|t| t.accepts.iter().map(|h| set_named(registry, h)))
        .collect();
    let title = &game.title;
    // The sets refused as not drawn right yet ([`tables_unplaced`]), by
    // MAME's name: named apart, never among the sets the page takes.
    let mut unplaced = Vec::new();
    for t in &takers {
        let Ok(text) = read(&dir.join(&t.id).join("asset-map.toml")) else {
            continue;
        };
        let Ok(map) = gp_assets::arcade::ArcadeMap::parse(&text) else {
            continue;
        };
        for h in &t.accepts {
            if let Some(n) = mame_name(h).filter(|n| !tables_unplaced(&map, n).is_empty()) {
                unplaced.push(n);
            }
        }
    }
    // The sets the page takes, in every refusal's `expected` (build-page.sh
    // writes the page's romset line from them): those not drawn right yet
    // left out.
    let expected: Vec<Named> = expected
        .into_iter()
        .filter(|n| !mame_name(&n.sha256).is_some_and(|m| unplaced.contains(&m)))
        .collect();
    let sets = sets_sentence(&expected);
    let takes = match unplaced.as_slice() {
        [] => format!("This page takes a MAME romset zip of {title}: {sets}."),
        [one] => format!(
            "This page takes a MAME romset zip of {title}: {sets} ({one} isn't drawn right yet)."
        ),
        many => format!(
            "This page takes a MAME romset zip of {title}: {sets} ({} aren't drawn right yet).",
            sets_sentence_names(many)
        ),
    };
    if !gp_arcade::romset::looks_like_zip(rom) {
        return Ok(refuse(
            format!("This file isn't a zip. {takes}"),
            expected,
            Named::default(),
        ));
    }
    let set = match gp_arcade::Romset::from_zip(rom) {
        Ok(s) => s,
        Err(gp_arcade::romset::RomError::Zip(_, why)) => {
            return Ok(refuse(
                format!("This zip can't be read ({why}). {takes}"),
                expected,
                Named::default(),
            ));
        }
        Err(gp_arcade::romset::RomError::Unknown {
            nearest, have, of, ..
        }) if have > 0 => {
            let missing = missing_chips(rom, nearest);
            let named_set = gp_arcade::romset::set(nearest).map_or(nearest, |s| s.title);
            return Ok(refuse(
                format!(
                    "This zip isn't a whole {title} romset: it has {have} of the {of} chips of {nearest} ({named_set}) and lacks {}. {takes}",
                    if missing.is_empty() {
                        "the rest".to_owned()
                    } else {
                        missing.join(", ")
                    }
                ),
                expected,
                Named::default(),
            ));
        }
        Err(gp_arcade::romset::RomError::Unknown { .. }) => {
            return Ok(refuse(
                format!("This zip holds no chip of a {title} romset. {takes}"),
                expected,
                Named::default(),
            ));
        }
        Err(e) => {
            return Ok(refuse(
                format!("This zip isn't a whole {title} romset ({e}). {takes}"),
                expected,
                Named::default(),
            ));
        }
    };
    let got = set_named(registry, &set.identity);
    let Some(taker) = takers.iter().find(|t| {
        t.accepts
            .iter()
            .any(|h| h.eq_ignore_ascii_case(&set.identity))
    }) else {
        return Ok(refuse(
            format!(
                "This is {} ({}), a set this page doesn't take. {takes}",
                set.name,
                got.name.as_deref().unwrap_or(set.name)
            ),
            expected,
            got,
        ));
    };
    let tdir = dir.join(&taker.id);
    let map_text = read(&tdir.join("asset-map.toml"))?;
    let map = gp_assets::arcade::ArcadeMap::parse(&map_text)
        .map_err(|e| format!("{}: asset-map.toml: {e}", taker.id))?;
    let unplaced = tables_unplaced(&map, set.name);
    if !unplaced.is_empty() {
        return Ok(refuse(
            format!(
                "This is {} ({}), which this page doesn't draw right yet: the places of its program's {} {} aren't recorded, so its screen, colours or poses would come out wrong. {takes}",
                set.name,
                got.name.as_deref().unwrap_or(set.name),
                if unplaced.len() == 1 {
                    "table"
                } else {
                    "tables"
                },
                tables_words(&unplaced)
            ),
            expected,
            got,
        ));
    }
    let (files, _) = gp_assets::arcade::lift(&decoded(&set), &map, &map_text)
        .map_err(|e| format!("lifting {}: {e}", set.name))?;
    let lifted = out.join("lifted");
    for f in &files {
        write(&lifted.join(&f.name), &f.bytes)?;
    }
    let model_file = tdir.join(gp_model::MODEL_FILE);
    let load: Option<Load> = crate::read_load(&tdir)?;
    let model_text = match &load {
        Some(l) => l.model_text(&read(&model_file)?, &map.source)?,
        None => read(&model_file)?,
    };
    let mut model =
        Model::parse(&model_text).map_err(|e| format!("{}: {e}", model_file.display()))?;
    let (looks, mut looks_skipped) =
        crate::template_looks(&tdir.join(gp_present::looks::THEMES_DIR), &model.model.name);
    let names = load.as_ref().map(|_| model.clone());
    let full = load.is_none();
    if let Some(l) = &load {
        l.apply(&mut model)?;
    }
    if let Some(levels) = &game.levels {
        crate::play_only(&mut model, levels);
    }
    let prepped = crate::prep_model(
        model,
        &tdir,
        &lifted,
        out,
        &gp_theme::Resolved::none(),
        None,
        &looks,
        true,
        names.as_ref(),
    )?;
    looks_skipped.extend(prepped.skipped);
    let mut looks_partial = prepped.partial;
    // Finished (large-art) looks: a theme installed beside the game, its
    // rules naming the board's pictures (docs/formats/board-art.md,
    // "Themes"), offered as on the NES.
    crate::add_ready_looks(
        &tdir.join(gp_present::looks::THEMES_DIR),
        out,
        (true, full),
        &mut looks_skipped,
        &mut looks_partial,
    )?;
    let mut written = Vec::new();
    crate::list(out, out, &mut written)?;
    written.retain(|f| !f.starts_with("lifted/") && f != "result.json");
    written.sort();
    let info = serde_json::json!({
        "game": game.game,
        "template": taker.id,
        "plays": load.as_ref().map(|l| l.plays.clone()),
        "template_name": got.name,
        "rom_name": got.name,
        "set": set.name,
        "match": "exact",
        "program_share": 1.0,
        "warning": serde_json::Value::Null,
        "assets": map.assets.iter().map(|a| serde_json::json!({"name": a.name, "from": "same"})).collect::<Vec<_>>(),
        "compared": [],
        "ignored": set.extras,
    });
    Ok(Outcome::Lifted {
        sha256: set.identity.clone(),
        files: written,
        found: Some(info),
        looks_skipped,
        looks_partial,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A zip of stored entries, made here: made-up bytes only.
    pub(crate) fn zip(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, data) in files {
            let at = out.len() as u32;
            let crc = 0u32;
            let head = |sig: &[u8], central: bool| {
                let mut h = sig.to_vec();
                if central {
                    h.extend(20u16.to_le_bytes());
                }
                h.extend(20u16.to_le_bytes());
                h.extend(0u16.to_le_bytes());
                h.extend(0u16.to_le_bytes()); // stored
                h.extend(0u32.to_le_bytes()); // time and date
                h.extend(crc.to_le_bytes());
                h.extend((data.len() as u32).to_le_bytes());
                h.extend((data.len() as u32).to_le_bytes());
                h.extend((name.len() as u16).to_le_bytes());
                h.extend(0u16.to_le_bytes());
                if central {
                    // Comment length, disk, internal and external
                    // attributes, then the local header's place.
                    h.extend([0u8; 10]);
                    h.extend(at.to_le_bytes());
                }
                h.extend(name.as_bytes());
                h
            };
            out.extend(head(b"PK\x03\x04", false));
            out.extend(*data);
            central.extend(head(b"PK\x01\x02", true));
        }
        let start = out.len() as u32;
        out.extend(&central);
        out.extend(b"PK\x05\x06");
        out.extend([0u8; 4]);
        out.extend((files.len() as u16).to_le_bytes());
        out.extend((files.len() as u16).to_le_bytes());
        out.extend((central.len() as u32).to_le_bytes());
        out.extend(start.to_le_bytes());
        out.extend(0u16.to_le_bytes());
        out
    }

    /// An arcade game folder as build-lift.sh writes one, with the sets'
    /// identities the board knows: tapper as the template, rbtapper loaded
    /// only to play. No chip and no picture: only names and hashes.
    pub(crate) fn folder(d: &Path) {
        folder_with(d, "\n[asset.at]\nrbtapper = 0\n");
    }

    /// [`folder`] with `at` after its one table: its places, or none.
    fn folder_with(d: &Path, at: &str) {
        let id = |n: &str| gp_arcade::romset::set(n).unwrap().identity();
        std::fs::write(
            d.join(crate::GAME_FILE),
            r#"{"game": "tapper", "title": "Tapper", "templates": ["tapper", "rbtapper"]}"#,
        )
        .unwrap();
        let manifest = format!(
            "[source]\naccepts = [\"{}\"]\n\n[[loads]]\nsource = \"rbtapper\"\naccepts = [\"{}\"]\n",
            id("tapper"),
            id("rbtapper")
        );
        let map = format!(
            "schema = 1\nplatform = \"arcade\"\nsource = \"tapper\"\nsets = [\"tapper\", \"rbtapper\"]\n\n[[asset]]\nname = \"one\"\nformat = \"byte-rows\"\nregion = \"maincpu\"\noffset = 0\ncount = 1\nwidth = 1\n{at}"
        );
        for t in ["tapper", "rbtapper"] {
            std::fs::create_dir_all(d.join(t)).unwrap();
            std::fs::write(d.join(t).join("template.toml"), &manifest).unwrap();
            std::fs::write(d.join(t).join("asset-map.toml"), &map).unwrap();
        }
        std::fs::write(
            d.join("rbtapper").join(crate::LOAD_FILE),
            r#"{"plays": "tapper"}"#,
        )
        .unwrap();
    }

    fn refused(rom: &[u8]) -> Refused {
        let dir = tempfile::tempdir().unwrap();
        folder(dir.path());
        let out = dir.path().join("out");
        match crate::lift_rom(rom, dir.path(), &out).unwrap() {
            Outcome::Refused(r) => {
                assert!(!out.exists(), "nothing is written for a refused zip");
                r
            }
            Outcome::Lifted { .. } => panic!("lifted"),
        }
    }

    #[test]
    fn a_file_that_is_not_a_zip_is_refused_naming_the_game_and_its_sets() {
        let r = refused(b"NES\x1a not a romset");
        assert!(
            r.error.starts_with("This file isn't a zip. This page takes a MAME romset zip of Tapper: tapper or rbtapper."),
            "{}",
            r.error
        );
        assert_eq!(r.expected.len(), 2);
        assert_eq!(r.expected[1].id.as_deref(), Some("rbtapper"));
    }

    #[test]
    fn a_zip_of_no_chip_of_the_game_is_refused() {
        let r = refused(&zip(&[("readme.txt", b"made up"), ("a.bin", &[7u8; 64])]));
        assert!(
            r.error
                .starts_with("This zip holds no chip of a Tapper romset."),
            "{}",
            r.error
        );
    }

    #[test]
    fn a_zip_that_cannot_be_read_is_refused() {
        let mut z = zip(&[("a.bin", &[1u8; 16])]);
        // The central directory's first entry no longer starts as one.
        let at = z.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
        z[at + 2] = 9;
        let r = refused(&z);
        assert!(r.error.starts_with("This zip can't be read"), "{}", r.error);
    }

    /// The owner's romsets, when they are on this machine: each lifts
    /// onto its folder's model, and a set with a chip taken out is refused
    /// naming the chip.
    #[test]
    fn a_whole_set_is_known_and_a_set_with_a_chip_out_is_refused_by_chip() {
        let Some(path) = gp_arcade::romset::local_romset("rbtapper") else {
            eprintln!("skip: no rbtapper.zip in $GP_ROM_DIR");
            return;
        };
        let bytes = std::fs::read(path).unwrap();
        let files = gp_arcade::romset::read_zip(&bytes).unwrap();
        let set = gp_arcade::romset::set("rbtapper").unwrap();
        // Every file but the first chip, stored again.
        let first = set.chips[0];
        let kept: Vec<(&str, &[u8])> = files
            .iter()
            .filter(|(_, d)| {
                gp_arcade::sha1::sha1(d)
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>()
                    != first.sha1
            })
            .map(|(n, d)| (n.as_str(), d.as_slice()))
            .collect();
        let r = refused(&zip(&kept));
        assert!(
            r.error.contains("isn't a whole Tapper romset")
                && r.error.contains(&format!("lacks {}", first.name)),
            "{}",
            r.error
        );
        // The whole set: taken by the rbtapper folder. The made-up map
        // lifts one byte into a folder of the test's, and the model is missing, so the lift
        // stops there: the set was known and taken.
        let dir = tempfile::tempdir().unwrap();
        folder(dir.path());
        let e = crate::lift_rom(&bytes, dir.path(), &dir.path().join("out"))
            .err()
            .expect("the made-up folder has no model");
        assert!(e.contains("rbtapper") && e.contains("model.toml"), "{e}");
        // The same set where the map records no place of its tables:
        // refused by name, before anything is lifted.
        let dir = tempfile::tempdir().unwrap();
        folder_with(dir.path(), "");
        let out = dir.path().join("out");
        match crate::lift_rom(&bytes, dir.path(), &out).unwrap() {
            Outcome::Refused(r) => {
                assert!(
                    r.error.starts_with("This is rbtapper (")
                        && r.error.contains("which this page doesn't draw right yet: the places of its program's table one aren't recorded")
                        && r.error.ends_with("This page takes a MAME romset zip of Tapper: tapper (rbtapper isn't drawn right yet)."),
                    "{}",
                    r.error
                );
                assert!(!out.exists(), "nothing is written");
            }
            Outcome::Lifted { .. } => panic!("lifted"),
        }
    }

    #[test]
    fn a_set_not_drawn_right_yet_is_named_apart_from_the_sets_taken() {
        let dir = tempfile::tempdir().unwrap();
        folder_with(dir.path(), "");
        match crate::lift_rom(b"not a zip", dir.path(), &dir.path().join("out")).unwrap() {
            Outcome::Refused(r) => {
                assert_eq!(
                    r.error,
                    "This file isn't a zip. This page takes a MAME romset zip of Tapper: tapper (rbtapper isn't drawn right yet)."
                );
                let taken: Vec<_> = r.expected.iter().map(|n| n.id.as_deref()).collect();
                assert_eq!(taken, [Some("tapper")], "the sets taken leave it out");
            }
            Outcome::Lifted { .. } => panic!("lifted"),
        }
    }

    /// A map of one program table, its places given by `at`.
    fn map_with(at: &str) -> gp_assets::arcade::ArcadeMap {
        gp_assets::arcade::ArcadeMap::parse(&format!(
            "schema = 1\nplatform = \"arcade\"\nsource = \"tapper\"\nsets = [\"tapper\", \"rbtapper\", \"tapperg\", \"tappera\"]\n\n[[asset]]\nname = \"one\"\nformat = \"byte-rows\"\nregion = \"maincpu\"\noffset = 0\ncount = 1\nwidth = 1\n{at}"
        ))
        .unwrap()
    }

    #[test]
    fn a_set_whose_program_tables_have_no_place_is_unplaced() {
        let none = map_with("");
        // Another program, no place recorded: unplaced.
        assert_eq!(tables_unplaced(&none, "rbtapper"), ["one"]);
        assert_eq!(tables_unplaced(&none, "tappera"), ["one"]);
        // The map's own set, and a set with the same program chips (tapperg
        // is tapper's program with other sprites): read where the map says.
        assert!(tables_unplaced(&none, "tapper").is_empty());
        assert!(same_program("tapper", "tapperg"));
        assert!(tables_unplaced(&none, "tapperg").is_empty());
        // A place recorded for the set: placed.
        let placed = map_with("\n[asset.at]\nrbtapper = 0x10\n");
        assert!(tables_unplaced(&placed, "rbtapper").is_empty());
        assert_eq!(tables_unplaced(&placed, "tappera"), ["one"]);
        // A map that reads no program table has nothing to place.
        let pictures = gp_assets::arcade::ArcadeMap::parse(
            "schema = 1\nplatform = \"arcade\"\nsource = \"tapper\"\nsets = [\"tapper\", \"rbtapper\"]\n\n[[asset]]\nname = \"tiles\"\nformat = \"mcr-tiles\"\nregion = \"gfx1\"\nfirst = 0\ncount = 4\n",
        )
        .unwrap();
        assert!(tables_unplaced(&pictures, "rbtapper").is_empty());
    }

    /// A made-up map of five program tables and a picture region: a set
    /// with some of its tables' places recorded and others missing is
    /// unplaced, naming each missing one; a place given as the table's own
    /// offset (the set keeps it there) counts; a table of a graphics region
    /// needs none.
    #[test]
    fn a_set_with_some_places_recorded_and_others_missing_is_unplaced_naming_them() {
        let table = |name: &str, at: &str| {
            format!(
                "\n[[asset]]\nname = \"{name}\"\nformat = \"byte-rows\"\nregion = \"maincpu\"\noffset = 0x100\ncount = 1\nwidth = 1\n{at}"
            )
        };
        let text = format!(
            "schema = 1\nplatform = \"arcade\"\nsource = \"tapper\"\nsets = [\"tapper\", \"rbtapper\", \"tappera\"]\n\n[[asset]]\nname = \"tiles\"\nformat = \"mcr-tiles\"\nregion = \"gfx1\"\nfirst = 0\ncount = 4\n{}{}{}{}{}",
            table("moved", "\n[asset.at]\nrbtapper = 0x180\ntappera = 0x190\n"),
            table("same_place", "\n[asset.at]\nrbtapper = 0x100\n"),
            table("walk", ""),
            table("wait", "\n[asset.at]\ntappera = 0x1A0\n"),
            table("idle", ""),
        );
        let map = gp_assets::arcade::ArcadeMap::parse(&text).unwrap();
        assert_eq!(tables_unplaced(&map, "rbtapper"), ["walk", "wait", "idle"]);
        assert_eq!(
            tables_unplaced(&map, "tappera"),
            ["same_place", "walk", "idle"]
        );
        assert!(tables_unplaced(&map, "tapperg").is_empty());
        assert_eq!(
            tables_words(&tables_unplaced(&map, "rbtapper")),
            "walk, wait and idle"
        );
        let many: Vec<String> = (1..=5).map(|i| format!("t{i}")).collect();
        assert_eq!(tables_words(&many), "t1, t2, t3 and 2 more");
        assert_eq!(tables_words(&many[..1]), "t1");
        // Every place recorded for the set: placed.
        let all = format!(
            "schema = 1\nplatform = \"arcade\"\nsource = \"tapper\"\nsets = [\"tapper\", \"rbtapper\"]\n{}{}",
            table("moved", "\n[asset.at]\nrbtapper = 0x180\n"),
            table("walk", "\n[asset.at]\nrbtapper = 0x100\n"),
        );
        let all = gp_assets::arcade::ArcadeMap::parse(&all).unwrap();
        assert!(tables_unplaced(&all, "rbtapper").is_empty());
    }
}
