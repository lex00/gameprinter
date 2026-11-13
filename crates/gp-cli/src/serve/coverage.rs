//! Coverage for the inspector (docs/coverage.md, docs/inspector.md): a
//! recording's coverage, exactly as `gp coverage <id> --no-save --json`
//! reports it, and the source listing its executed bytes are drawn over.
//!
//! Neither writes to the store. Coverage is read from the store when an
//! earlier `gp coverage` stored it with the same definition, and measured
//! by replay otherwise, which
//! takes a few seconds for a long recording; `gp coverage --store` stores
//! every recording's, so the inspector reads them at once.
//!
//! Ported from game-printer's serve/coverage.rs (wave 8); Super Mario
//! Bros.'s generated listing was that game's and is cut, as gp-interp's
//! other games were.

use std::path::PathBuf;

use serde_json::json;

use super::{Ctx, Fail, Params, Route};
use crate::mechanic::http::{Request, Response};

pub(super) fn routes() -> Vec<Route> {
    vec![
        Route {
            path: "/api/recordings/{id}/coverage",
            about: "The recording's coverage: executed ROM bytes, RAM writes, screens and inputs",
            handler: coverage,
        },
        Route {
            path: "/api/recordings/{id}/listing",
            about: "The source listing of the recording's game, with each line's PRG ROM bytes",
            handler: listing,
        },
    ]
}

fn id(params: &Params) -> &str {
    params.get("id").map_or("", String::as_str)
}

fn coverage(ctx: &Ctx, _: &Request, params: &Params) -> Result<Response, Fail> {
    let opened = super::recordings::open(ctx, id(params))?;
    let out = crate::coverage::served(&opened.entry.id.to_hex(), None, &ctx.paths)?;
    Ok(super::recordings::outcome(out))
}

/// Where a game's ld65 debug file is, and how it's made.
enum Place {
    /// Built from the repo's source into Cargo's target folder; the ROM
    /// next to it must be the recording's.
    Built(&'static str, &'static str),
}

/// Games with a listing, by the ID `labels::game_of` gives, with the recipe
/// that makes it.
const LISTINGS: &[(&str, Place, &str)] = &[(
    "mosshop",
    Place::Built("mosshop/mosshop.dbg", "mosshop/mosshop.nes"),
    "just mosshop",
)];

fn listing(ctx: &Ctx, _: &Request, params: &Params) -> Result<Response, Fail> {
    let opened = super::recordings::open(ctx, id(params))?;
    let sha256 = opened.sha256().to_owned();
    let game = super::labels::game_of(ctx, &sha256);
    let missing = |why: String| {
        Ok(Response::json(&json!({
            "id": opened.entry.id, "game": game, "missing": why, "listing": null,
        })))
    };
    let Some((_, place, recipe)) = game
        .as_deref()
        .and_then(|g| LISTINGS.iter().find(|(id, ..)| *id == g))
    else {
        return missing(match &game {
            Some(g) => format!("no source listing is known for {g}"),
            None => "the recording's ROM isn't a known source or baseline".into(),
        });
    };
    let path: PathBuf = match place {
        Place::Built(dbg, rom) => {
            let Some(target) = target_dir(ctx) else {
                return missing(format!(
                    "the listing is built in the repo by `{recipe}`, and gp serve isn't running in it"
                ));
            };
            let built = std::fs::read(target.join(rom))
                .ok()
                .and_then(|b| gp_evidence::source::identify_nes(&b).ok());
            match built {
                None => return missing(format!("there's no built ROM; run `{recipe}`")),
                Some(image) if image.sha256 != sha256 => {
                    return missing(format!(
                        "the built ROM isn't the one this recording ran (another version of the game); run `{recipe}` at the commit that made it"
                    ));
                }
                Some(_) => target.join(dbg),
            }
        }
    };
    if !path.is_file() {
        return missing(format!("{} doesn't exist; run `{recipe}`", path.display()));
    }
    // A source that's gone or changed since the build is the same as no
    // listing: the recipe makes it again.
    let listing = match gp_coverage::listing::read(&path) {
        Ok(l) => l,
        Err(
            e @ (gp_coverage::listing::ListingError::Io { .. }
            | gp_coverage::listing::ListingError::Changed { .. }),
        ) => return missing(format!("{e}; run `{recipe}`")),
        Err(e) => {
            return Err(Fail {
                status: 500,
                message: e.to_string(),
            });
        }
    };
    Ok(Response::json(&json!({
        "id": opened.entry.id, "game": game, "missing": null, "listing": listing,
    })))
}

/// Cargo's target folder, where `just mosshop` builds: $CARGO_TARGET_DIR,
/// or target/ in the repo.
fn target_dir(ctx: &Ctx) -> Option<PathBuf> {
    match std::env::var_os("CARGO_TARGET_DIR").filter(|v| !v.is_empty()) {
        Some(dir) => Some(PathBuf::from(dir)),
        None => Some(ctx.repo.join("target")),
    }
}
