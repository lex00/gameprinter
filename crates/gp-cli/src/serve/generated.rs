//! The clips the tool has for a run (game-printer's serve/generated.rs,
//! docs/show.md, "Clips made by the tool"). game-printer made them from a
//! capture run's recordings into the run's corpus; in gameprinter a game's
//! corpus is already in the games checkout (`corpus.toml` and `inputs/`,
//! made by `gp corpus import` from the mechanic sessions), so these are the
//! game's corpus clips, each from its own start. The strip groups them by
//! the mechanic session they came from.
//!
//! The start pictures and the player's state bytes at each start are made
//! the first time they're asked for and kept in `generated/starts/`. Both
//! show the game's own art or come from its ROM, so they stay in $GP_HOME.
//! Marks from the old editor (kept, skipped, starred) in
//! `$GP_HOME/work/serve/GAME/generated/marks.json` are still read by the
//! background work's scoring; nothing writes them any more.
//!
//! | Endpoint | What |
//! |---|---|
//! | `GET /api/capture/{run}/generated/starts/{start}/picture.png` | The original's picture at a start (REFINE's queue AS PICTURES) |

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use super::game::{Run, is_name, read_json, rom_of, run_dir};
use super::{Ctx, Fail, Params, Route};
use crate::mechanic::http::{Request, Response};

pub(super) fn routes() -> Vec<Route> {
    vec![Route {
        path: "/api/capture/{run}/generated/starts/{start}/picture.png",
        about: "The original's picture at a start",
        handler: picture,
    }]
}

/// A corpus clip as the server sees it.
pub(crate) struct Made {
    pub id: String,
    pub name: String,
    /// The mechanic session it came from: `walk` of `dk.walk.001`.
    pub group: String,
    pub frames: u64,
}

/// The session a clip ID names: the middle of `GAME.SESSION.NNN`.
fn group_of(id: &str) -> String {
    let parts: Vec<&str> = id.split('.').collect();
    match parts.as_slice() {
        [_, mid @ .., last] if !mid.is_empty() && last.chars().all(|c| c.is_ascii_digit()) => {
            mid.join(".")
        }
        _ => "corpus".into(),
    }
}

/// The game's corpus clips, in the corpus's order.
pub(crate) fn made(run: &Run) -> Vec<Made> {
    let Ok(set) = crate::clipset::read_game(&run.game) else {
        return Vec::new();
    };
    set.clips
        .into_iter()
        .map(|r| Made {
            group: group_of(&r.id),
            id: r.id,
            name: r.name,
            frames: r.frames,
        })
        .collect()
}

fn marks_file(run: &Run) -> PathBuf {
    run.dir.join("generated").join("marks.json")
}

/// The marks on the tool's clips, by clip ID. Unmarked clips are kept.
pub(crate) fn marks(run: &Run) -> BTreeMap<String, String> {
    serde_json::from_value(read_json(&marks_file(run))).unwrap_or_default()
}

/// A corpus clip's input from power-on and its start frame.
pub(crate) fn clip_log(run: &Run, id: &str) -> Result<(gp_evidence::InputLog, usize), Fail> {
    let c = crate::clip_score::corpus_clip(&run.game, id)
        .map_err(|e| Fail::not_found(format!("{e:#}")))?;
    Ok(c.log()?)
}

fn start_dir(run: &Run) -> PathBuf {
    run.dir.join("generated").join("starts")
}

/// The player at a start: its state bytes and picture, made once and kept.
fn made_start(ctx: &Ctx, run: &Run, id: &str) -> Result<(PathBuf, PathBuf), Fail> {
    if !is_name(id) {
        return Err(Fail::not_found("no such start"));
    }
    let key = crate::clipset::file_key(id);
    let dir = start_dir(run);
    let (state, png) = (
        dir.join(format!("{key}.state")),
        dir.join(format!("{key}.png")),
    );
    if state.is_file() && png.is_file() {
        return Ok((state, png));
    }
    let (log, at) = clip_log(run, id)?;
    let rom = rom_of(ctx, run)?;
    let mut head = log.clone();
    head.frames.truncate(at);
    // The player the page plays this platform with, so the state bytes are
    // ones its player restores: gp-play for the NES, gp-arcade-play for an
    // arcade set, with the set's own screen size.
    let (bytes, picture) = if run.platform() == crate::roms::Platform::Arcade {
        let mut p =
            gp_arcade_play::Player::replay(&rom, &head.to_text()).map_err(anyhow::Error::msg)?;
        p.run_to(at as u64).map_err(anyhow::Error::msg)?;
        let bytes = p.save().map_err(anyhow::Error::msg)?;
        let (w, h) = (
            gp_arcade::video::WIDTH as u32,
            gp_arcade::video::HEIGHT as u32,
        );
        (bytes, encode_png(p.rgba(), w, h)?)
    } else {
        let mut p = gp_play::Player::replay(&rom, &head.to_text()).map_err(anyhow::Error::msg)?;
        p.run_to(at as u64).map_err(anyhow::Error::msg)?;
        let bytes = p.save().map_err(anyhow::Error::msg)?;
        (bytes, encode_png(p.rgba(), 256, 240)?)
    };
    fs::create_dir_all(&dir)?;
    fs::write(&state, bytes)?;
    fs::write(&png, picture)?;
    Ok((state, png))
}

/// An RGBA picture as a PNG.
pub(crate) fn encode_png(rgba: &[u8], w: u32, h: u32) -> Result<Vec<u8>, Fail> {
    let mut out = Vec::new();
    {
        let mut e = png::Encoder::new(&mut out, w, h);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        let mut wr = e.write_header().map_err(|e| anyhow::anyhow!("{e}"))?;
        wr.write_image_data(rgba)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
    }
    Ok(out)
}

fn picture(ctx: &Ctx, _req: &Request, params: &Params) -> Result<Response, Fail> {
    let run = run_dir(ctx, params)?;
    let id = params.get("start").cloned().unwrap_or_default();
    let (_, png) = made_start(ctx, &run, &id)?;
    Ok(Response::bytes("image/png", fs::read(png)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clip_is_grouped_by_its_session() {
        assert_eq!(group_of("dk.climb-ladder.006"), "climb-ladder");
        assert_eq!(group_of("dk.m50-girder3.001"), "m50-girder3");
        assert_eq!(group_of("plain"), "corpus");
    }
}
