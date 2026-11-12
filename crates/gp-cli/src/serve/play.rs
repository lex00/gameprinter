//! The live player and the ROMs it plays (game-printer's serve/play.rs,
//! the part free play needs). The page runs `gp_play.wasm`, built by
//! scripts/web/build-play.sh into target/play-web (`just play-web`), or
//! `$GP_PLAY_WASM`.
//!
//! | Endpoint | What |
//! |---|---|
//! | `GET /api/play/gp_play.wasm` | The player module |
//! | `GET /api/play/roms` | The games' ROMs that are on this machine, and whether the player is built |
//! | `GET /api/play/roms/{sha256}` | A ROM's bytes, found by hash in $GP_ROM_DIR; kept in the page's memory only |
//!
//! game-printer's recorded play sessions (`/api/play/sessions`, which
//! record into the store) are not here yet.

use std::path::PathBuf;

use serde_json::{Value, json};

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
            path: "/api/play/roms",
            about: "The games' ROMs on this machine, and whether the player is built",
            handler: roms,
        },
        Route {
            path: "/api/play/roms/{sha256}",
            about: "A ROM's bytes by its SHA-256, from $GP_ROM_DIR; kept in the page's memory only",
            handler: rom,
        },
    ]
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

/// The scored ROM of each game whose ROM is here.
fn playable(ctx: &Ctx) -> Vec<Value> {
    let mut out = Vec::new();
    for name in super::game::all_runs(ctx) {
        let Ok(run) = super::game::run_named(ctx, &name) else {
            continue;
        };
        if let Ok(rom) = super::game::rom_of(ctx, &run) {
            let sha = gp_evidence::source::identify_nes(&rom)
                .map(|i| i.sha256)
                .unwrap_or_default();
            out.push(json!({"sha256": sha, "name": name, "source": name, "template": Value::Null}));
        }
    }
    out
}

fn roms(ctx: &Ctx, _req: &Request, _params: &Params) -> Result<Response, Fail> {
    Ok(Response::json(&json!({
        "roms": playable(ctx),
        "wasm": wasm_path(ctx).is_file(),
        "scratch_store": ctx.paths.home.join("play").join("store"),
        "real_store": ctx.paths.store,
    })))
}

fn rom(ctx: &Ctx, _req: &Request, params: &Params) -> Result<Response, Fail> {
    let sha = params.get("sha256").map(String::as_str).unwrap_or("");
    if sha.len() != 64 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Fail::bad_request("a ROM is named by its SHA-256"));
    }
    let mut found = crate::roms::find(&[sha.to_owned()], &[], &ctx.paths.roms)
        .map_err(|e| Fail::not_found(format!("{e:#}")))?;
    let r = found
        .remove(sha)
        .ok_or_else(|| Fail::not_found(format!("no ROM with hash {sha} in $GP_ROM_DIR")))?;
    Ok(Response::bytes("application/octet-stream", r.bytes))
}
