//! `gp clip presses`: a played clip saved as a corpus clip. The clip's
//! input from power-on is split at its start into the two canonical
//! `.presses` files a corpus plan names (`inputs/ID.start.presses` and
//! `inputs/ID.presses`), the `[[clip]]` added to the plan's `corpus.toml`
//! and its recipe-set hash made again. NES and arcade clips alike: an
//! arcade clip's files carry the `arcade SYSTEM COLUMNS` line and name
//! arcade buttons (crate::clipset::Presses).
//!
//! The files are read back as the scorer reads them and must give the same
//! input, frame for frame. With the ROM at hand, the original replays them
//! and the state hash at the clip's start and end is printed; a mechanic
//! session's clip records its start's hash, which must be the one replayed.

use std::fs;
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use gp_evidence::{InputLog, Paths};
use gp_runtime::Runtime as _;
use serde_json::json;

use crate::Outcome;
use crate::clipset::{self as cs, Presses};
use crate::original::OriginalRuntime;

/// Where the clip comes from.
pub(crate) enum From<'a> {
    /// A mechanic session's clip: `MECHANIC/N` of the game's sessions.
    Session(&'a str),
    /// An input log from power-on, the clip being `frames` frames from
    /// `start` (to the log's end when not given).
    Log {
        file: &'a Path,
        start: u64,
        frames: Option<u64>,
    },
}

/// What the clip's `[[clip]]` says besides its input.
pub(crate) struct About<'a> {
    pub name: &'a str,
    pub why: &'a str,
    pub level: &'a str,
    pub source: &'a str,
}

pub(crate) fn run(
    paths: &Paths,
    game: &Path,
    id: &str,
    from: &From,
    about: &About,
    rom: Option<&Path>,
) -> Result<Outcome> {
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        || id.is_empty()
    {
        bail!("a clip ID is letters, digits, dots, dashes and underscores: {id}");
    }
    let (log, start, recorded) = match from {
        From::Session(spec) => session_clip(paths, game, spec)?,
        From::Log {
            file,
            start,
            frames,
        } => {
            let text =
                fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
            let mut log =
                InputLog::parse(&text).map_err(|e| anyhow::anyhow!("{}: {e}", file.display()))?;
            let end = match frames {
                Some(n) => start + n,
                None => log.frames.len() as u64,
            };
            if end > log.frames.len() as u64 || end <= *start {
                bail!(
                    "{} has {} frames: no clip of frames {start} to {end}",
                    file.display(),
                    log.frames.len()
                );
            }
            log.frames.truncate(end as usize);
            (log, *start as usize, None)
        }
    };
    let (head, own) = split(&log, start)?;
    let inputs = game.join("inputs");
    let start_rel = format!("inputs/{id}.start.presses");
    let own_rel = format!("inputs/{id}.presses");
    for rel in [&start_rel, &own_rel] {
        if game.join(rel).exists() {
            bail!("{} is already there: a clip ID is used once", rel);
        }
    }
    let plan_file = game.join(crate::corpus_score::FILE);
    let plan_text = fs::read_to_string(&plan_file)
        .with_context(|| format!("reading {}", plan_file.display()))?;
    if crate::plan::load(game)?.is_none() {
        bail!("{} isn't a corpus plan (format 1.0.0)", plan_file.display());
    }
    if plan_text.contains(&format!("id = {}", toml_str(id))) {
        bail!("{} already has a clip {id}", plan_file.display());
    }
    fs::create_dir_all(&inputs)?;
    fs::write(game.join(&start_rel), cs::presses_text(&head))?;
    fs::write(game.join(&own_rel), cs::presses_text(&own))?;

    let mut stanza = format!(
        "\n[[clip]]\nid = {}\nname = {}\nsource = {}\nstart = {}\npresses = {}\nframes = {}\n",
        toml_str(id),
        toml_str(about.name),
        toml_str(about.source),
        toml_str(&start_rel),
        toml_str(&own_rel),
        own.frames
    );
    if !about.why.is_empty() {
        stanza += &format!("why = {}\n", toml_str(about.why));
    }
    if !about.level.is_empty() {
        stanza += &format!("level = {}\n", toml_str(about.level));
    }
    let mut text = plan_text.clone();
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text += &stanza;
    fs::write(&plan_file, &text)?;
    let loaded = match crate::plan::load(game) {
        Ok(Some(l)) => l,
        other => {
            // Put the plan back as it was: the files alone name no clip.
            fs::write(&plan_file, &plan_text)?;
            let _ = fs::remove_file(game.join(&start_rel));
            let _ = fs::remove_file(game.join(&own_rel));
            bail!(
                "the plan with the clip added doesn't read: {:?}",
                other.err()
            );
        }
    };
    fs::write(&plan_file, with_recipes(&text, &loaded.hash)?)?;

    // Read back as the scorer reads it: the same input, frame for frame.
    let clip = crate::clip_score::corpus_clip(game, id)?;
    let (back, back_start) = clip.log()?;
    if back_start != start || back.frames != log.frames || back.arcade != log.arcade {
        bail!("the clip's files don't read back as the input they were made from");
    }

    let mut j = json!({
        "clip": id,
        "start": start,
        "frames": own.frames,
        "files": [start_rel, own_rel],
        "recipes": loaded.hash,
        "arcade": log.arcade.as_ref().map(|a| &a.system),
    });
    let mut lines = vec![
        format!(
            "wrote {start_rel} ({start} frames) and {own_rel} ({} frames), and clip {id} in corpus.toml",
            own.frames
        ),
        format!("read back: the same input from power-on, the clip starting at frame {start}"),
        format!("recipes = {}", loaded.hash),
    ];
    match crate::clip_score::rom_of(paths, game, rom) {
        Ok(bytes) => {
            let (at_start, at_end) = replay(&bytes, &back, start)?;
            if let Some(want) = &recorded
                && *want != at_start
            {
                bail!(
                    "the replay's state at frame {start} is {at_start}; the session's clip recorded {want}"
                );
            }
            lines.push(format!(
                "replayed: state {} at its start (frame {start}){}, {} at its end (frame {})",
                &at_start[..16],
                if recorded.is_some() {
                    ", the session's clip's"
                } else {
                    ""
                },
                &at_end[..16],
                log.frames.len()
            ));
            j["state_start"] = json!(at_start);
            j["state_end"] = json!(at_end);
        }
        Err(e) => lines.push(format!("not replayed: {e:#}")),
    }
    Ok(Outcome::ok(j, lines.join("\n")))
}

/// The input from power-on split at `start`: the presses to it and the
/// clip's own.
fn split(log: &InputLog, start: usize) -> Result<(Presses, Presses)> {
    if start > log.frames.len() {
        bail!(
            "the clip starts at frame {start}, past its input's {} frames",
            log.frames.len()
        );
    }
    let part = |frames: &[gp_evidence::input_log::InputFrame]| {
        let mut l = log.clone();
        l.frames = frames.to_vec();
        cs::presses_of(&l).context(
            "the input presses reset or a third or fourth controller, which spans can't say",
        )
    };
    Ok((part(&log.frames[..start])?, part(&log.frames[start..])?))
}

/// A session's clip `MECHANIC/N` of the game named by the package's
/// folder: its input from power-on, its start and the state hash it
/// recorded there.
fn session_clip(
    paths: &Paths,
    game: &Path,
    spec: &str,
) -> Result<(InputLog, usize, Option<String>)> {
    let name = game
        .canonicalize()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .context("the game's folder has no name")?;
    let (mechanic, n) = spec
        .split_once('/')
        .and_then(|(m, n)| Some((m, n.parse::<u64>().ok()?)))
        .with_context(|| format!("--from-session is MECHANIC/N, such as serve/1: {spec}"))?;
    let s = crate::mechanic::session::Session::open(paths, &name, mechanic)?;
    let (text, start) = crate::mechanic::paths::full_log(&paths.home, &s.dir, n)?;
    let log = InputLog::parse(&text).map_err(|e| anyhow::anyhow!("clip {n}: {e}"))?;
    let meta: toml::Table = toml::from_str(&fs::read_to_string(
        s.dir.join(format!("clips/{n:03}.toml")),
    )?)?;
    let state = meta
        .get("state")
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    Ok((log, start, state))
}

/// The original's state hash after `start` frames and at the end.
fn replay(rom: &[u8], log: &InputLog, start: usize) -> Result<(String, String)> {
    let mut o = OriginalRuntime::of_rom(rom)?;
    let inputs = crate::run::frames(log);
    o.step(&inputs[..start]).map_err(anyhow::Error::msg)?;
    let at_start = o.state_hex()?;
    o.step(&inputs[start..]).map_err(anyhow::Error::msg)?;
    Ok((at_start, o.state_hex()?))
}

fn toml_str(s: &str) -> String {
    toml::Value::String(s.to_owned()).to_string()
}

/// The plan's text with its `[corpus]` table's `recipes` set to `hash`.
fn with_recipes(text: &str, hash: &str) -> Result<String> {
    let mut out = String::with_capacity(text.len());
    let mut table = "";
    let mut done = false;
    for line in text.split_inclusive('\n') {
        let t = line.trim();
        if t.starts_with('[') {
            table = if t == "[corpus]" { "corpus" } else { "other" };
        }
        if table == "corpus" && !done && t.starts_with("recipes") && t.contains('=') {
            out += &format!("recipes = {}\n", toml_str(hash));
            done = true;
        } else {
            out += line;
        }
    }
    if !done {
        bail!("the plan's [corpus] has no recipes line");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAN: &str = "[corpus]\nformat = \"1.0.0\"\nname = \"t\"\nversion = \"0.1.0\"\nrecipes = \"b3:0\"\n\n[target]\ngame = \"t\"\nrom = \"00\"\n";

    /// An arcade clip from a log is split into its two files with the
    /// arcade line, added to the plan with a new recipe-set hash, and
    /// reads back as the same input; an NES log's files have no arcade
    /// line. A second clip of the same ID is refused.
    #[test]
    fn a_log_becomes_a_corpus_clip_and_reads_back() {
        let d = tempfile::tempdir().unwrap();
        let game = d.path().join("t");
        fs::create_dir_all(&game).unwrap();
        fs::write(game.join("corpus.toml"), PLAN).unwrap();
        let log = "gp-arcade-input 0\nsystem tapper\ncolumns CSRLDUT\n10 .......\n4 C......\n6 .......\n3 .S.....\n5 .....UT\n";
        let file = d.path().join("a.input");
        fs::write(&file, log).unwrap();
        let paths = Paths {
            home: d.path().join("home"),
            roms: d.path().join("roms"),
            ..Paths::from_env().unwrap()
        };
        let about = About {
            name: "a serve",
            why: "",
            level: "",
            source: "played",
        };
        let from = From::Log {
            file: &file,
            start: 20,
            frames: None,
        };
        let out = run(&paths, &game, "t.serve.1", &from, &about, None).unwrap();
        assert_eq!(out.json["start"], 20);
        let start = fs::read_to_string(game.join("inputs/t.serve.1.start.presses")).unwrap();
        assert_eq!(
            start,
            "gp-presses 0\nframes 20\narcade tapper CSRLDUT\ncoin@10-13\n"
        );
        let own = fs::read_to_string(game.join("inputs/t.serve.1.presses")).unwrap();
        assert_eq!(
            own,
            "gp-presses 0\nframes 8\narcade tapper CSRLDUT\nstart1@0-2\nup+tap@3-7\n"
        );
        let plan = fs::read_to_string(game.join("corpus.toml")).unwrap();
        assert!(!plan.contains("b3:0"), "{plan}");
        assert!(plan.contains("[[clip]]\nid = \"t.serve.1\""), "{plan}");
        let again = run(&paths, &game, "t.serve.1", &from, &about, None);
        assert!(again.is_err());

        let nes = d.path().join("n.log");
        fs::write(
            &nes,
            "gp-input-log 0\ncontrollers 1\nP ........\nP R......A\n",
        )
        .unwrap();
        let from = From::Log {
            file: &nes,
            start: 1,
            frames: None,
        };
        run(&paths, &game, "t.nes.1", &from, &about, None).unwrap();
        let own = fs::read_to_string(game.join("inputs/t.nes.1.presses")).unwrap();
        assert_eq!(own, "gp-presses 0\nframes 1\nright+a@0-0\n");
    }
}
