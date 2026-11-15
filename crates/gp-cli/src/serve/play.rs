//! The live player (game-printer's serve/play.rs, the part free play
//! needs). The page runs `gp_play.wasm`, built by
//! scripts/web/build-play.sh into target/play-web (`just play-web`), or
//! `$GP_PLAY_WASM`; for an arcade game `gp_arcade_play.wasm`
//! (`scripts/web/build-play.sh --arcade`), or `$GP_ARCADE_PLAY_WASM`.
//!
//! | Endpoint | What |
//! |---|---|
//! | `GET /api/play/gp_play.wasm` | The NES player module |
//! | `GET /api/play/gp_arcade_play.wasm` | The arcade board's player module |
//!
//! game-printer's recorded play sessions (`/api/play/sessions`, which
//! record into the store) are not here yet.

use std::path::PathBuf;

use super::{Ctx, Fail, Params, Route};
use crate::mechanic::http::{Request, Response};

pub(super) fn routes() -> Vec<Route> {
    vec![
        Route {
            path: "/api/play/gp_play.wasm",
            about: "The live player module (gp_play.wasm)",
            handler: wasm,
        },
        Route {
            path: "/api/play/gp_arcade_play.wasm",
            about: "The arcade board's live player module (gp_arcade_play.wasm)",
            handler: arcade_wasm,
        },
    ]
}

/// `$VAR`, else the repo's target/play-web/FILE.
fn wasm_path(ctx: &Ctx, var: &str, file: &str) -> PathBuf {
    std::env::var_os(var).filter(|v| !v.is_empty()).map_or_else(
        || ctx.repo.join("target/play-web").join(file),
        PathBuf::from,
    )
}

fn serve_wasm(ctx: &Ctx, var: &str, file: &str, build: &str) -> Result<Response, Fail> {
    let p = wasm_path(ctx, var, file);
    let bytes = std::fs::read(&p).map_err(|_| {
        Fail::not_found(format!(
            "the player isn't built: run {build}, or set {var}; looked for {}",
            p.display()
        ))
    })?;
    Ok(Response::bytes("application/wasm", bytes))
}

fn wasm(ctx: &Ctx, _req: &Request, _params: &Params) -> Result<Response, Fail> {
    serve_wasm(
        ctx,
        "GP_PLAY_WASM",
        "gp_play.wasm",
        "scripts/web/build-play.sh (just play-web)",
    )
}

fn arcade_wasm(ctx: &Ctx, _req: &Request, _params: &Params) -> Result<Response, Fail> {
    serve_wasm(
        ctx,
        "GP_ARCADE_PLAY_WASM",
        "gp_arcade_play.wasm",
        "scripts/web/build-play.sh --arcade (just play-web-arcade)",
    )
}
