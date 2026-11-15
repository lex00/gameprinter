//! `gp query clip:GAME:CLIP`: a corpus clip's play read as a recording is,
//! with no recording made. The clip's input from power-on (as `gp clip
//! log` writes it) is played on the original, NES or arcade, from
//! power-on, and the selectors are read at the frames asked for. Frames
//! count from power-on, as a recording's do, so a frame of the clip's
//! start is the same frame in both.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use gp_runtime::{InputFrame, Runtime as _};
use serde_json::json;

use crate::Outcome;
use crate::recordings::{Row, entry_json, entry_line, parse_frames};

/// The prefix that names a clip in place of a recording's ID.
pub(crate) const PREFIX: &str = "clip:";

/// The game folder and the clip's ID of `clip:GAME:CLIP`: GAME is a
/// package folder or a game's name in the games checkout.
fn split(spec: &str) -> Result<(PathBuf, String)> {
    let rest = spec.strip_prefix(PREFIX).unwrap_or(spec);
    let Some((game, clip)) = rest.split_once(':') else {
        bail!("a clip is clip:GAME:CLIP (clip:tapper:tapper.gameover.002), not {spec}");
    };
    let dir = Path::new(game);
    if dir.join("package.toml").is_file() {
        return Ok((dir.to_owned(), clip.to_owned()));
    }
    let games = crate::roms_cmd::games_dir(None)
        .context("no games checkout found to find the game in: set GP_GAMES_DIR")?;
    let dir = crate::labels::game_folder(&games, game)
        .with_context(|| format!("no game {game} in {}", games.display()))?;
    Ok((dir, clip.to_owned()))
}

/// The selectors read on the clip's play: one frame (`frames` None, at
/// `frame`) or a range, every frame or only those that change.
pub(crate) fn query(
    spec: &str,
    frame: Option<u64>,
    frames: Option<&str>,
    selectors: &[String],
    changed: bool,
    rom: Option<&Path>,
    paths: &gp_evidence::Paths,
) -> Result<Outcome> {
    let (game, clip) = split(spec)?;
    let c = crate::clip_score::corpus_clip(&game, &clip)?;
    let (log, start) = c.log()?;
    let last = log.frames.len() as u64;
    let range = match (frames, frame) {
        (Some(f), _) => parse_frames(f, last)?,
        (None, Some(f)) if f <= last => f..f + 1,
        (None, Some(f)) => bail!("frame {f} is past the clip's last frame, {last}"),
        (None, None) => bail!("gp query needs --frame or --frames"),
    };
    let rom = crate::clip_score::rom_of(paths, &game, rom)?;
    let mut o = crate::original::OriginalRuntime::of_rom(&rom)?;
    let input = |f: &gp_evidence::input_log::InputFrame| InputFrame {
        reset: f.reset,
        ..InputFrame::pads(f.pads)
    };
    // Power-on is frame 0; the log's frame i is the step to frame i + 1.
    let mut rows = Vec::new();
    for (i, f) in log.frames.iter().enumerate().take(range.end as usize) {
        if i as u64 >= range.start {
            rows.push(read(&o, i as u64, selectors)?);
        }
        o.step(&[input(f)]).map_err(anyhow::Error::msg)?;
    }
    if range.end > last {
        rows.push(read(&o, last, selectors)?);
    }
    let shown = if changed {
        crate::recordings::changed_rows(rows)
    } else {
        rows
    };
    let one = frames.is_none();
    let text: Vec<String> = if one {
        shown
            .iter()
            .flat_map(|r| r.entries.iter().map(entry_line))
            .collect()
    } else {
        shown
            .iter()
            .map(|r| {
                let cells: Vec<String> = r.entries.iter().map(entry_line).collect();
                format!("frame {}: {}", r.frame, cells.join(", "))
            })
            .collect()
    };
    Ok(Outcome::ok(
        json!({
            "clip": clip,
            "game": game,
            "clip_start": start,
            "frames_from_power_on": last,
            "read_from": "the clip's play from power-on",
            "rows": shown
                .iter()
                .map(|r| json!({
                    "frame": r.frame,
                    "entries": r.entries.iter().map(entry_json).collect::<Vec<_>>(),
                }))
                .collect::<Vec<_>>(),
        }),
        text.join("\n"),
    ))
}

fn read(o: &crate::original::OriginalRuntime, frame: u64, selectors: &[String]) -> Result<Row> {
    let mut entries = Vec::new();
    for s in selectors {
        entries.extend(o.query(s).map_err(anyhow::Error::msg)?);
    }
    Ok(Row { frame, entries })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A clip is named with its game, by folder or name; anything else is
    /// refused naming the form.
    #[test]
    fn a_clip_is_named_with_its_game() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("package.toml"), "").unwrap();
        let spec = format!("clip:{}:cab.play.001", dir.path().display());
        let (game, clip) = split(&spec).unwrap();
        assert_eq!(
            (game.as_path(), clip.as_str()),
            (dir.path(), "cab.play.001")
        );
        let e = split("clip:cab.play.001").unwrap_err().to_string();
        assert!(e.contains("clip:GAME:CLIP"), "{e}");
    }
}
