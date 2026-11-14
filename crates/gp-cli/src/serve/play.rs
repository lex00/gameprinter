//! The live player (game-printer's serve/play.rs, the part free play
//! needs). The page runs `gp_play.wasm`, built by
//! scripts/web/build-play.sh into target/play-web (`just play-web`), or
//! `$GP_PLAY_WASM`.
//!
//! | Endpoint | What |
//! |---|---|
//! | `GET /api/play/gp_play.wasm` | The player module |
//!
//! game-printer's recorded play sessions (`/api/play/sessions`, which
//! record into the store) are not here yet.

use std::path::PathBuf;

use super::{Ctx, Fail, Params, Route};
use crate::mechanic::http::{Request, Response};

pub(super) fn routes() -> Vec<Route> {
    vec![Route {
        path: "/api/play/gp_play.wasm",
        about: "The live player module (gp_play.wasm)",
        handler: wasm,
    }]
}

/// `$GP_PLAY_WASM`, else the repo's target/play-web/gp_play.wasm.
fn wasm_path(ctx: &Ctx) -> PathBuf {
    std::env::var_os("GP_PLAY_WASM")
        .filter(|v| !v.is_empty())
        .map_or_else(
            || ctx.repo.join("target/play-web/gp_play.wasm"),
            PathBuf::from,
        )
}

fn wasm(ctx: &Ctx, _req: &Request, _params: &Params) -> Result<Response, Fail> {
    let p = wasm_path(ctx);
    let bytes = std::fs::read(&p).map_err(|_| {
        Fail::not_found(format!(
            "the player isn't built: run scripts/web/build-play.sh (just play-web), or set GP_PLAY_WASM; looked for {}",
            p.display()
        ))
    })?;
    Ok(Response::bytes("application/wasm", bytes))
}
