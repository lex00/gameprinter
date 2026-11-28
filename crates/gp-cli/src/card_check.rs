//! `gp model gate GAME --patch FILE --clip C --frames A-B --card-check`:
//! whether a gate card's LOOK AT IT would play, checked before the card is
//! shown (docs/loop.md, a gate's card). gp_run runs it before it puts a
//! model gate card on the person's page, and refuses the card when it
//! fails. On 2026-11-25 three of four open cards of an arcade game named
//! clips the page refused ("an NES input log can't play on an arcade
//! board"), one a row clip of another set, and one asked frames 600 to 640
//! of a 467 frame clip, so its look stood still on the last frame.
//!
//! Three checks, each said apart:
//! - clip: the clip is found, and its log, as the page is handed it, plays
//!   on the game's board and set (an arcade log of the same set and columns
//!   on an arcade board, an NES log on the NES);
//! - frames: the frames asked are inside the clip's own frames;
//! - model: the patch applies to the game as the games checkout's HEAD
//!   holds it, as the look copies it, model.toml parses, and the module
//!   builds from the hook code.

use std::path::Path;

use anyhow::{Context as _, Result, bail};
use gp_evidence::Paths;
use gp_evidence::input_log::InputLog;
use serde_json::json;

use crate::Outcome;

/// The clip `spec` names, as the page plays it: its log from power-on and
/// the frame its own frames start at. `ROW/N` is clip N of the quest-log
/// row's session (served as it was saved); an id is a corpus clip of the
/// game, given in the board's spelling as `/api/screen/work-clip` gives it.
/// A bare N is a clip of the step's row, which gp_run gives as ROW/N.
fn clip_log(
    home: &Path,
    game_dir: &Path,
    game: &str,
    rom: &[u8],
    spec: &str,
) -> std::result::Result<(InputLog, usize), String> {
    let spec = spec.trim();
    if let Some((row, n)) = spec.split_once('/')
        && let Ok(n) = n.parse::<u64>()
    {
        let dir = home.join("mechanics").join(game).join(row);
        if !dir.join(format!("clips/{n:03}.toml")).is_file() {
            return Err(format!("row {row} has no clip {n}"));
        }
        let (log, start) = crate::mechanic::paths::full_log(home, &dir, n)
            .map_err(|e| format!("clip {n} of row {row}: {e:#}"))?;
        let log = InputLog::parse(&log).map_err(|e| format!("clip {n} of row {row}: {e}"))?;
        return Ok((log, start));
    }
    if spec.parse::<u64>().is_ok() {
        return Err(format!(
            "clip {spec} is a clip of the step's row, and no row is known here: give it as ROW/{spec}"
        ));
    }
    let c = crate::clip_score::corpus_clip(game_dir, spec)
        .map_err(|_| format!("{game}'s corpus has no clip {spec}"))?;
    let (log, start) = c.log().map_err(|e| format!("{e:#}"))?;
    let log = crate::findings_clip::board_log(rom, log).map_err(|e| format!("{e:#}"))?;
    Ok((log, start))
}

/// Why `log` can't play on `rom`'s board, as the page's player refuses it
/// (gp-arcade-play's `Player::script`), or None.
fn wont_play(rom: &[u8], game: &str, log: &InputLog) -> Option<String> {
    let board = match crate::mechanic::probe::Board::of_rom(rom) {
        Ok(b) => b,
        Err(e) => return Some(format!("{game}'s ROM can't be read: {e:#}")),
    };
    match (&board.arcade, &log.arcade) {
        (None, None) => None,
        (Some(b), Some(l)) if b == l => None,
        (Some(b), Some(l)) => Some(format!(
            "its log is of {} with columns {}, and {game}'s board is {} with {}",
            l.system, l.columns, b.system, b.columns
        )),
        (Some(_), None) => {
            Some("its log is an NES input log, which can't play on an arcade board".into())
        }
        (None, Some(l)) => Some(format!(
            "its log is an arcade log of {}, which can't play on the NES",
            l.system
        )),
    }
}

/// Why frames `frames` (A-B or A, from the clip's start) aren't inside a
/// clip of `len` frames, or None.
fn frames_outside(frames: &str, len: usize) -> Option<String> {
    let num = |s: &str| s.trim().parse::<usize>().ok();
    let (a, b) = match frames.split_once('-') {
        Some((a, b)) => (num(a), num(b)),
        None => (num(frames), num(frames)),
    };
    let (Some(a), Some(b)) = (a, b) else {
        return Some(format!("frames {frames} aren't A-B or A"));
    };
    if b < a {
        return Some(format!("frames {frames} end before they start"));
    }
    if len == 0 {
        return Some("the clip has no frames of its own".into());
    }
    (b >= len).then(|| {
        format!(
            "frames {frames} aren't inside the clip, which has {len} frames (0 to {})",
            len - 1
        )
    })
}

/// Why the model as `patch` would leave the game in `game_dir` doesn't
/// build, made in a folder under `$GP_HOME/work/card-check`, or None.
fn model_trouble(paths: &Paths, game_dir: &Path, game: &str, patch: &Path) -> Option<String> {
    let made = || -> Result<()> {
        let work = paths.home.join("work").join("card-check");
        std::fs::create_dir_all(&work)?;
        let tree = tempfile::tempdir_in(&work)?;
        let copy = crate::mechanic::ask_look::patched(tree.path(), game, game_dir, patch)?;
        let model = copy.join("model.toml");
        if model.is_file() {
            gp_model::Model::parse(&std::fs::read_to_string(&model)?)
                .map_err(|e| anyhow::anyhow!("model.toml: {e}"))?;
        }
        crate::module::built_for(&paths.home, &copy)
            .context("building the changed model's module")?;
        Ok(())
    };
    made().err().map(|e| format!("{e:#}"))
}

/// The three checks' results: (check, why it failed or None).
pub(crate) fn checks(
    paths: &Paths,
    game_dir: &Path,
    rom: &[u8],
    clip: &str,
    frames: &str,
    patch: &Path,
) -> Vec<(&'static str, Option<String>)> {
    let game = game_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (clip_why, frames_why) = match clip_log(&paths.home, game_dir, &game, rom, clip) {
        Err(why) => (
            Some(why),
            Some(format!(
                "clip {clip} wasn't found, so its frames weren't counted"
            )),
        ),
        Ok((log, start)) => (
            wont_play(rom, &game, &log).map(|w| format!("clip {clip}: {w}")),
            frames_outside(frames, log.frames.len().saturating_sub(start))
                .map(|w| format!("clip {clip}: {w}")),
        ),
    };
    vec![
        ("clip", clip_why),
        ("frames", frames_why),
        ("model", model_trouble(paths, game_dir, &game, patch)),
    ]
}

/// `gp model gate --card-check`: exit 0 when the card's look would play,
/// 1 with each check that failed.
pub(crate) fn run(a: &crate::gate::GateArgs, paths: &Paths) -> Result<Outcome> {
    let game_dir = gp_evidence::paths::real_path(&a.game);
    if !game_dir.is_dir() {
        bail!("{}: No such folder", a.game.display());
    }
    let (Some(patch), Some(clip), Some(frames)) = (&a.patch, &a.clip, &a.frames) else {
        bail!(
            "--card-check needs --patch, --clip and --frames: the card's look is the clip's frames on the model as the patch leaves it"
        );
    };
    let rom = crate::clip_score::rom_of(paths, &game_dir, None)?;
    let done = checks(paths, &game_dir, &rom, clip, frames, patch);
    let failed: Vec<String> = done
        .iter()
        .filter_map(|(k, w)| w.as_ref().map(|w| format!("{k}: {w}")))
        .collect();
    let mut text = String::new();
    for (k, w) in &done {
        match w {
            None => text.push_str(&format!("OK {k}\n")),
            Some(w) => text.push_str(&format!("FAIL {k}: {w}\n")),
        }
    }
    let json = json!({
        "look_plays": failed.is_empty(),
        "checks": done.iter().map(|(k, w)| json!({"check": k, "ok": w.is_none(), "why": w})).collect::<Vec<_>>(),
        "refused": (!failed.is_empty()).then(|| format!(
            "the card's LOOK AT IT wouldn't play, so the card isn't shown: {}",
            failed.join("; ")
        )),
    });
    Ok(Outcome {
        json,
        text,
        exit: u8::from(!failed.is_empty()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A card's look is refused naming which check failed: a log of
    /// another set or an NES log on an arcade board (a made-up game's zip),
    /// an arcade log on the NES (clip), frames past the clip's end
    /// (frames); and each passes when it fits.
    #[test]
    fn a_cards_look_is_checked_before_it_is_shown() {
        // The frames: 600-640 of a 467 frame clip is outside it.
        let why = frames_outside("600-640", 467).unwrap();
        assert!(why.contains("467 frames (0 to 466)"), "{why}");
        assert_eq!(frames_outside("400-466", 467), None);
        assert_eq!(frames_outside("5", 6), None);
        assert!(frames_outside("6", 6).is_some());
        assert!(frames_outside("9-4", 20).is_some());
        assert!(frames_outside("x", 20).is_some());
        assert!(frames_outside("0", 0).is_some());
        // The clip's log against the board: an NES log plays on the NES
        // and not on an arcade board, an arcade log only on its own set.
        let nes_rom = {
            let mut r = b"NES\x1a\x01\x01\x00\x00".to_vec();
            r.resize(16 + 0x4000 + 0x2000, 0);
            r
        };
        let nes = InputLog::new(1);
        assert_eq!(wont_play(&nes_rom, "g", &nes), None);
        let other = InputLog::new_arcade("hilltopb", "CSRLDUT");
        assert!(
            wont_play(&nes_rom, "g", &other)
                .unwrap()
                .contains("can't play on the NES")
        );
        let set = "madeupcard";
        let zip = gp_arcade::testrom::known_made_up_game(set);
        let why = wont_play(&zip, "g", &nes).unwrap();
        assert!(why.contains("NES input log"), "{why}");
        let why = wont_play(&zip, "g", &other).unwrap();
        assert!(why.contains("hilltopb") && why.contains(set), "{why}");
        let own = InputLog::new_arcade(set, "CSRLDUT");
        assert_eq!(wont_play(&zip, "g", &own), None);
    }

    /// A bare clip number names no row here, and a row clip that isn't
    /// there is said so: both are the clip check's.
    #[test]
    fn a_clip_the_page_cant_find_fails_the_clip_check() {
        let d = tempfile::tempdir().unwrap();
        let game = d.path().join("g");
        std::fs::create_dir_all(&game).unwrap();
        let why = clip_log(d.path(), &game, "g", &[], "3").unwrap_err();
        assert!(why.contains("ROW/3"), "{why}");
        let why = clip_log(d.path(), &game, "g", &[], "walk/2").unwrap_err();
        assert_eq!(why, "row walk has no clip 2");
        let why = clip_log(d.path(), &game, "g", &[], "g.attract.001").unwrap_err();
        assert!(why.contains("no clip g.attract.001"), "{why}");
    }
}
