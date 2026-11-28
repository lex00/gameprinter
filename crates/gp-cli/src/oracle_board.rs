//! `gp oracle board GAME`: the checks of an arcade game against its own
//! ROM, run from the game's folder in the games checkout
//! (docs/oracle-mame.md, "Checks of a game against its ROM"). The checks
//! are gp_arcade::checks; this finds what each needs in the game's folder
//! (roms.toml, oracle/mame-captures.toml's `[[replay]]` rows, harvest.toml),
//! the set's zip in the ROM folder by its identity, and the captures under
//! `$GP_HOME/work/oracle/NAME`. A check whose set, capture or file isn't on
//! this machine is skipped with why. The tool's own tests never run them
//! on a real game: they belong to the game's folder.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde_json::json;

use gp_arcade::Romset;
use gp_arcade::checks;
use gp_arcade::oracle::{Capture, InputLog};
use gp_evidence::Paths;

use crate::Outcome;
use crate::oracle_captures::Captures;

/// Which checks to run; none chosen is all of them.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Which {
    /// The board against each `[[replay]]` capture (frames, AY writes,
    /// samples, the watchdog), snapshots, and the frame event's polls.
    pub board: bool,
    /// The sound board alone against the whole machine.
    pub sound: bool,
    /// The disassembler's lengths and flow against a trace.
    pub dasm: bool,
    /// The static harvest against harvest.toml.
    pub harvest: bool,
}

impl Which {
    fn all_if_none(self) -> Self {
        if self.board || self.sound || self.dasm || self.harvest {
            self
        } else {
            Self {
                board: true,
                sound: true,
                dasm: true,
                harvest: true,
            }
        }
    }
}

/// One check's line: its name, and Some(true) passed, Some(false) failed,
/// None skipped, with what it said.
struct Line {
    check: String,
    ok: Option<bool>,
    said: String,
}

struct Game<'a> {
    dir: PathBuf,
    rom: Option<&'a Path>,
    paths: &'a Paths,
}

impl Game<'_> {
    /// The known set `name`, loaded from its zip; Err(why) to skip.
    fn set(&self, name: &str) -> Result<std::result::Result<Romset, String>> {
        let Some(info) = gp_arcade::romset::set(name) else {
            return Ok(Err(format!(
                "{} lists no set {name}",
                self.dir.join("roms.toml").display()
            )));
        };
        let Some(zip) = crate::oracle_mame::find_zip(&info.identity(), self.rom, self.paths)?
        else {
            return Ok(Err(format!(
                "no {name} romset in {}",
                self.paths.roms.display()
            )));
        };
        let set = Romset::load_zip(&zip).map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(Ok(set))
    }

    /// The first of the game's sets (in roms.toml's order) on this machine.
    fn first_set(&self, sets: &[String]) -> Result<std::result::Result<Romset, String>> {
        let mut why = Vec::new();
        for s in sets {
            match self.set(s)? {
                Ok(r) => return Ok(Ok(r)),
                Err(w) => why.push(w),
            }
        }
        Ok(Err(if why.is_empty() {
            "the game's roms.toml lists no arcade set".into()
        } else {
            why.join("; ")
        }))
    }

    /// The `[[replay]]` captures oracle/mame-captures.toml lists, as (name,
    /// input log path); Err(why) when there are none.
    fn replays(&self) -> Result<std::result::Result<Vec<(String, PathBuf)>, String>> {
        let file = self.dir.join(crate::oracle_captures::FILE);
        if !file.is_file() {
            return Ok(Err(format!("no {}", file.display())));
        }
        let c = Captures::load(&file)?;
        if c.replays.is_empty() {
            return Ok(Err(format!("{} lists no [[replay]]", file.display())));
        }
        Ok(Ok(c
            .replays
            .iter()
            .map(|r| (r.name.clone(), c.root.join(&r.input)))
            .collect()))
    }
}

fn read_log(path: &Path) -> Result<InputLog> {
    let text = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    InputLog::parse(&text).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))
}

fn put(lines: &mut Vec<Line>, check: impl Into<String>, r: checks::Checked) {
    let (ok, said) = match r {
        Ok(s) => (Some(true), s),
        Err(s) => (Some(false), s),
    };
    lines.push(Line {
        check: check.into(),
        ok,
        said,
    });
}

fn skip(lines: &mut Vec<Line>, check: impl Into<String>, why: String) {
    lines.push(Line {
        check: check.into(),
        ok: None,
        said: why,
    });
}

/// `gp oracle board`: the chosen checks on the game in `dir`. Exits 1 when
/// one fails, 2 when every one was skipped.
pub(crate) fn run(dir: &Path, which: Which, rom: Option<&Path>, paths: &Paths) -> Result<Outcome> {
    let which = which.all_if_none();
    let roms_toml = dir.join("roms.toml");
    let text = fs::read_to_string(&roms_toml).with_context(|| {
        format!(
            "reading {}: give an arcade game's folder of the games checkout",
            roms_toml.display()
        )
    })?;
    let sets: Vec<String> = gp_arcade::romset::sets_from_roms_toml(&text)
        .map_err(|e| anyhow::anyhow!("{}: {e}", roms_toml.display()))?
        .iter()
        .map(|s| s.name.to_string())
        .collect();
    if sets.is_empty() {
        bail!(
            "{} lists no arcade set ([[source.chip]] rows): this isn't an arcade game's folder",
            roms_toml.display()
        );
    }
    gp_arcade::romset::add_games_dir(dir).map_err(anyhow::Error::msg)?;
    let game = Game {
        dir: dir.to_path_buf(),
        rom,
        paths,
    };
    let mut lines = Vec::new();

    if which.board {
        match game.replays()? {
            Err(why) => skip(&mut lines, "board replay", why),
            Ok(replays) => {
                for (name, input) in replays {
                    let label = format!("board replay {name}");
                    let cap_dir = paths.home.join("work").join("oracle").join(&name);
                    if !cap_dir.join("frames.tsv").is_file() {
                        skip(
                            &mut lines,
                            label,
                            format!(
                                "no capture at {}; make it with `just mame-record` (docs/oracle-mame.md)",
                                cap_dir.display()
                            ),
                        );
                        continue;
                    }
                    let log = read_log(&input)?;
                    match game.set(&log.system)? {
                        Err(why) => skip(&mut lines, label, why),
                        Ok(set) => {
                            let cap = Capture::load(&cap_dir).map_err(anyhow::Error::msg)?;
                            put(&mut lines, label, checks::replay(&set, &cap, &log));
                        }
                    }
                }
            }
        }
        match game.first_set(&sets)? {
            Err(why) => {
                skip(&mut lines, "board snapshots", why.clone());
                skip(&mut lines, "board polls", why);
            }
            Ok(set) => {
                put(&mut lines, "board snapshots", checks::snapshots(&set));
                put(&mut lines, "board polls", checks::polls(&set, 300));
            }
        }
    }

    if which.sound {
        match game.replays()? {
            Err(why) => skip(&mut lines, "sound board alone", why),
            Ok(replays) => {
                let log = read_log(&replays[0].1)?;
                match game.set(&log.system)? {
                    Err(why) => skip(&mut lines, "sound board alone", why),
                    Ok(set) => put(
                        &mut lines,
                        "sound board alone",
                        checks::sound_board_alone(&set, &log, 1500),
                    ),
                }
            }
        }
    }

    if which.dasm {
        match game.first_set(&sets)? {
            Err(why) => skip(&mut lines, "dasm", why),
            Ok(set) => put(&mut lines, "dasm", checks::dasm(&set, 900, 100_000)),
        }
    }

    if which.harvest {
        let file = dir.join("harvest.toml");
        match fs::read_to_string(&file) {
            Err(_) => skip(&mut lines, "harvest", format!("no {}", file.display())),
            Ok(text) => {
                let want = checks::HarvestWants::parse(&text).map_err(anyhow::Error::msg)?;
                match game.set(&want.set)? {
                    Err(why) => skip(&mut lines, "harvest", why),
                    Ok(set) => put(&mut lines, "harvest", checks::harvest(&set.maincpu, &want)),
                }
            }
        }
    }

    let count = |o: Option<bool>| lines.iter().filter(|l| l.ok == o).count();
    let (passed, failed, skipped) = (count(Some(true)), count(Some(false)), count(None));
    let mut text: Vec<String> = lines
        .iter()
        .map(|l| {
            let tag = match l.ok {
                Some(true) => "ok    ",
                Some(false) => "FAILED",
                None => "skip  ",
            };
            match l.ok {
                None => format!("{tag} {}  ({})", l.check, l.said),
                _ => format!("{tag} {}  {}", l.check, l.said),
            }
        })
        .collect();
    text.push(format!(
        "{passed} passed, {failed} failed, {skipped} skipped"
    ));
    let exit = if failed > 0 {
        1
    } else if passed == 0 {
        2
    } else {
        0
    };
    Ok(Outcome {
        json: json!({
            "game": dir,
            "checks": lines.iter().map(|l| json!({"check": l.check, "ok": l.ok, "said": l.said})).collect::<Vec<_>>(),
            "passed": passed, "failed": failed, "skipped": skipped,
        }),
        text: text.join("\n"),
        exit,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A made-up game folder (rooftop) whose roms.toml lists a made-up
    /// set, a home whose ROM folder has that set's zip when `with_zip`,
    /// and the paths.
    fn made_up(with_zip: bool) -> (tempfile::TempDir, PathBuf, Paths) {
        use std::io::Write as _;
        let d = tempfile::tempdir().unwrap();
        let game = d.path().join("games/rooftop");
        fs::create_dir_all(&game).unwrap();
        let (toml, files) = gp_arcade::testrom::made_up_set("rooftopboard");
        fs::write(game.join("roms.toml"), toml).unwrap();
        let paths = Paths::resolve(Some(d.path().join("home")), None, None).unwrap();
        if with_zip {
            let arcade = paths.roms.join("arcade");
            fs::create_dir_all(&arcade).unwrap();
            let mut out = std::io::Cursor::new(Vec::new());
            let mut z = zip::ZipWriter::new(&mut out);
            for (name, bytes) in &files {
                z.start_file(name.as_str(), zip::write::SimpleFileOptions::default())
                    .unwrap();
                z.write_all(bytes).unwrap();
            }
            z.finish().unwrap();
            fs::write(arcade.join("rooftopboard.zip"), out.into_inner()).unwrap();
        }
        (d, game, paths)
    }

    #[test]
    fn oracle_board_skips_every_check_without_the_romset() {
        let (_d, game, paths) = made_up(false);
        let out = run(&game, Which::default(), None, &paths).unwrap();
        assert_eq!(out.exit, 2, "{}", out.text);
        for check in [
            "board replay",
            "board snapshots",
            "board polls",
            "sound board alone",
            "dasm",
            "harvest",
        ] {
            assert!(
                out.text.contains(&format!("skip   {check}")),
                "{check}: {}",
                out.text
            );
        }
        assert!(
            out.text.contains("no rooftopboard romset in"),
            "{}",
            out.text
        );
        assert!(
            out.text.ends_with("0 passed, 0 failed, 6 skipped"),
            "{}",
            out.text
        );
    }

    #[test]
    fn oracle_board_holds_the_harvest_to_harvest_toml_on_the_sets_zip() {
        let (_d, game, paths) = made_up(true);
        fs::write(
            game.join("harvest.toml"),
            "set = \"rooftopboard\"\n\n[[fact]]\npath = \"no.such.fact\"\n",
        )
        .unwrap();
        let only = Which {
            harvest: true,
            ..Which::default()
        };
        let out = run(&game, only, None, &paths).unwrap();
        assert_eq!(out.exit, 1, "{}", out.text);
        assert!(
            out.text
                .contains("FAILED harvest  the harvest missed 1 of 1: no fact no.such.fact"),
            "{}",
            out.text
        );
        assert!(!out.text.contains("dasm"), "only the harvest: {}", out.text);
        // The snapshot check runs on the set found by its identity.
        let board = Which {
            board: true,
            ..Which::default()
        };
        let out = run(&game, board, None, &paths).unwrap();
        assert!(out.text.contains("ok     board snapshots"), "{}", out.text);
        assert!(out.text.contains("skip   board replay"), "{}", out.text);
    }

    #[test]
    fn oracle_board_refuses_a_folder_that_isnt_an_arcade_games() {
        let d = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(Some(d.path().join("home")), None, None).unwrap();
        let Err(err) = run(d.path(), Which::default(), None, &paths) else {
            panic!("a folder with no roms.toml was checked");
        };
        assert!(format!("{err:#}").contains("roms.toml"), "{err:#}");
        fs::write(d.path().join("roms.toml"), "").unwrap();
        let Err(err) = run(d.path(), Which::default(), None, &paths) else {
            panic!("a roms.toml with no arcade set was checked");
        };
        assert!(
            format!("{err:#}").contains("lists no arcade set"),
            "{err:#}"
        );
    }
}
