//! RAM labels for the inspector's labeled RAM watch (docs/inspector.md).
//!
//! A recording's game is its source's registry ID (found by the ROM's
//! hash in the games' roms.toml files). Its labels file is the game's
//! `labels.toml` ([`crate::labels`]). The inspector uses the RAM labels;
//! `gp labels` reads the routines and data tables too.
//!
//! Ported from game-printer's serve/labels.rs (wave 8). What gameprinter's
//! layout changed: a game's labels file is in its folder of the games
//! checkout (or Moss Hop's in this repo), where game-printer named each
//! game's file in a table and found built games by their baselines.

use std::path::PathBuf;

use serde_json::json;

use super::{Ctx, Fail, Params, Route};
use crate::labels::Label;
use crate::mechanic::http::{Request, Response};

pub(super) fn routes() -> Vec<Route> {
    vec![Route {
        path: "/api/recordings/{id}/labels",
        about: "RAM labels for the recording's game, when it has any",
        handler: labels,
    }]
}

/// The registry of the games checkout's ROM entries.
pub(crate) fn registry(ctx: &Ctx) -> Option<gp_evidence::registry::Registry> {
    gp_evidence::registry::Registry::load_games(&ctx.games).ok()
}

/// The recording's game: the ID of the ROM entry with its hash.
pub(crate) fn game_of(ctx: &Ctx, sha256: &str) -> Option<String> {
    registry(ctx)?.by_sha256(sha256).map(|k| k.id.clone())
}

/// A recording's game and its labels, or why it has none.
pub(crate) struct Loaded {
    pub game: Option<String>,
    pub file: Option<PathBuf>,
    pub missing: Option<String>,
    pub labels: Vec<Label>,
}

impl Loaded {
    /// The names of the labels covering `addr`.
    pub fn at(&self, addr: u16) -> Vec<&str> {
        self.labels
            .iter()
            .filter(|l| {
                addr >= l.addr && u32::from(addr) < u32::from(l.addr) + u32::from(l.size.max(1))
            })
            .map(|l| l.name.as_str())
            .collect()
    }
}

/// The labels of the game whose ROM has this hash.
pub(crate) fn load(ctx: &Ctx, sha256: &str) -> Result<Loaded, Fail> {
    let game = game_of(ctx, sha256);
    let located = match &game {
        None => Err("the recording's ROM isn't a ROM entry of any game's roms.toml".to_owned()),
        Some(g) => crate::labels::locate_in(g, Some(&ctx.repo), Some(&ctx.games)),
    };
    let (path, missing) = match located {
        Err(why) => (None, Some(why)),
        Ok(path) => (Some(path), None),
    };
    let labels = match &path {
        Some(p) => crate::labels::read(p)?.label,
        None => Vec::new(),
    };
    Ok(Loaded {
        game,
        file: path,
        missing,
        labels,
    })
}

fn labels(ctx: &Ctx, _: &Request, params: &Params) -> Result<Response, Fail> {
    let opened = super::recordings::open(ctx, params.get("id").map_or("", String::as_str))?;
    let l = load(ctx, opened.sha256())?;
    Ok(Response::json(&json!({
        "id": opened.entry.id,
        "game": l.game,
        "file": l.file,
        "missing": l.missing,
        "labels": l.labels,
    })))
}
