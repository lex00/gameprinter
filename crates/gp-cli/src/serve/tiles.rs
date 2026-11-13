//! Lifted tiles and palettes for the workspace's tile view
//! (docs/inspector.md, "Tiles"). A lifted tiles PNG holds color indices
//! 0 to 3, since a tile's colors depend on the palette it's drawn with
//! (docs/assets.md, "Tiles"). These endpoints serve the indices, the
//! lifted palettes and the NES's 64 colors, and the page applies a palette
//! when it draws; nothing is baked into the PNG.
//!
//! The files are read from the game's lifted directory,
//! `$GP_HOME/work/assets/<game>/`, through the same decoders `gp assets
//! pack` uses, so an edited file that wouldn't pack isn't shown either.

use std::path::{Path, PathBuf};

use gp_assets::{Asset, AssetMap};
use serde_json::{Value as Json, json};

use super::{Ctx, Fail, Params, Route};
use crate::mechanic::http::{Request, Response};

pub(super) fn routes() -> Vec<Route> {
    vec![
        Route {
            path: "/api/assets/{game}",
            about: "The game's lifted tile sets and palettes, from $GP_HOME/work/assets/<game>/",
            handler: list,
        },
        Route {
            path: "/api/assets/{game}/tiles/{name}",
            about: "A lifted tile set's color indices, one digit 0 to 3 a pixel, row by row",
            handler: tiles,
        },
    ]
}

/// A source ID: lower-case letters, digits and dashes, so it's one safe
/// path segment.
fn game(params: &Params) -> Result<&str, Fail> {
    let g = params.get("game").map_or("", String::as_str);
    let ok = !g.is_empty()
        && g.len() <= 64
        && g.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if !ok {
        return Err(Fail::bad_request(format!(
            "{g:?} isn't a source ID (lower-case letters, digits and dashes)"
        )));
    }
    Ok(g)
}

fn lifted_dir(ctx: &Ctx, game: &str) -> PathBuf {
    ctx.paths.home.join("work/assets").join(game)
}

/// The lifted directory's asset map, or `None` when there's no lift.
fn read_map(dir: &Path) -> Result<Option<AssetMap>, Fail> {
    let file = dir.join("assets.toml");
    let text = match std::fs::read_to_string(&file) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(anyhow::anyhow!("reading {}: {e}", file.display()).into()),
    };
    AssetMap::parse(&text)
        .map(Some)
        .map_err(|e| anyhow::anyhow!("{}: {e}", file.display()).into())
}

/// One asset's bytes, encoded from its lifted file as `gp assets pack`
/// encodes it, every span in order.
fn encoded(map: &AssetMap, asset: &Asset, dir: &Path) -> Result<Vec<u8>, String> {
    let one = AssetMap {
        assets: vec![asset.clone()],
        ..map.clone()
    };
    let read = |file: &str| std::fs::read(dir.join(file));
    let chunks = gp_assets::encode(&one, &read).map_err(|e| e.to_string())?;
    Ok(chunks.into_iter().flat_map(|c| c.bytes).collect())
}

/// A palette asset's palettes, four color numbers each.
fn palettes(map: &AssetMap, asset: &Asset, dir: &Path) -> Result<Vec<[u8; 4]>, String> {
    let bytes = encoded(map, asset, dir)?;
    Ok(bytes.as_chunks::<4>().0.to_vec())
}

fn list(ctx: &Ctx, _: &Request, params: &Params) -> Result<Response, Fail> {
    let game = game(params)?;
    let dir = lifted_dir(ctx, game);
    let Some(map) = read_map(&dir)? else {
        return Ok(Response::json(&json!({
            "game": game,
            "dir": dir,
            "missing": format!("{} holds no lift; `gp assets lift` makes one (docs/assets.md)", dir.display()),
            "tiles": [],
            "palettes": [],
        })));
    };
    let mut tiles = Vec::new();
    let mut pals = Vec::new();
    for asset in &map.assets {
        match asset {
            Asset::Chr {
                name,
                tiles: n,
                tiles_per_row,
                ..
            } => {
                let (w, h) = gp_assets::chr::image_size(*n, *tiles_per_row);
                tiles.push(json!({
                    "name": name,
                    "tiles": n,
                    "tiles_per_row": tiles_per_row,
                    "width": w,
                    "height": h,
                }));
            }
            Asset::Palette { name, .. } => {
                let entry: Json = match palettes(&map, asset, &dir) {
                    Ok(p) => json!({ "name": name, "palettes": p }),
                    Err(e) => json!({ "name": name, "palettes": [], "error": e }),
                };
                pals.push(entry);
            }
            _ => {}
        }
    }
    Ok(Response::json(&json!({
        "game": game,
        "dir": dir,
        "missing": null,
        "tiles": tiles,
        "palettes": pals,
    })))
}

fn tiles(ctx: &Ctx, _: &Request, params: &Params) -> Result<Response, Fail> {
    let game = game(params)?;
    let name = params.get("name").map_or("", String::as_str);
    let dir = lifted_dir(ctx, game);
    let map = read_map(&dir)?
        .ok_or_else(|| Fail::not_found(format!("{} holds no lift", dir.display())))?;
    let asset = map
        .assets
        .iter()
        .find(|a| matches!(a, Asset::Chr { name: n, .. } if n == name))
        .ok_or_else(|| Fail::not_found(format!("{game} has no tile set {name:?}")))?;
    let Asset::Chr {
        tiles: n,
        tiles_per_row,
        ..
    } = asset
    else {
        unreachable!("matched above");
    };
    let bytes = encoded(&map, asset, &dir).map_err(|e| Fail {
        status: 500,
        message: format!("{name}: {e}"),
    })?;
    let (w, h, pixels) = gp_assets::chr::decode(&bytes, *tiles_per_row);
    let digits: String = pixels.iter().map(|p| char::from(b'0' + (p & 3))).collect();
    Ok(Response::json(&json!({
        "game": game,
        "name": name,
        "tiles": n,
        "tiles_per_row": tiles_per_row,
        "width": w,
        "height": h,
        "pixels": digits,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_are_64_rgb_triples() {
        let c = gp_nes::video::colors();
        assert_eq!(c.len(), 64);
        // $0F is black, or nearly, and $30 near white.
        assert!(c[0x0f].iter().all(|&v| v < 0x10), "{:?}", c[0x0f]);
        assert!(c[0x30].iter().all(|&v| v > 0xe0), "{:?}", c[0x30]);
    }

    #[test]
    fn game_ids_are_one_safe_segment() {
        let p = |g: &str| Params::from([("game", g.to_owned())]);
        assert!(game(&p("smb1-ntsc-prg0")).is_ok());
        for bad in ["", "..", "a/b", "A", "x.y", &"a".repeat(65)] {
            assert!(game(&p(bad)).is_err(), "{bad:?}");
        }
    }

    /// A made-up two-tile game, lifted and read back.
    #[test]
    fn a_lift_is_listed_and_its_tiles_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let map_text = r#"
schema = 1
source = "made-up"
sha256 = "0000000000000000000000000000000000000000000000000000000000000000"

[[asset]]
format = "chr-2bpp"
name = "tiles"
region = "chr"
offset = 0
tiles = 2
tiles_per_row = 2

[[asset]]
format = "palette"
name = "palettes"
region = "prg"
offset = 0
count = 2
"#;
        let map = AssetMap::parse(map_text).unwrap();
        std::fs::write(dir.path().join("assets.toml"), map_text).unwrap();
        // Tile 0: plane 0 all set (index 1); tile 1: both planes (index 3).
        let mut chr = vec![0u8; 32];
        chr[..8].fill(0xff);
        chr[16..32].fill(0xff);
        let (w, h, px) = gp_assets::chr::decode(&chr, 2);
        std::fs::write(
            dir.path().join("tiles.png"),
            gp_assets::chr::write_png(w, h, &px),
        )
        .unwrap();
        std::fs::write(
            dir.path().join("palettes.toml"),
            "schema = 1\nasset = \"palettes\"\npalettes = [\n  [0x0F, 0x01, 0x02, 0x03],\n  [0x0F, 0x11, 0x12, 0x13],\n]\n",
        )
        .unwrap();
        let tiles = map.assets.iter().find(|a| a.name() == "tiles").unwrap();
        let bytes = encoded(&map, tiles, dir.path()).unwrap();
        assert_eq!(bytes, chr);
        let pal = map.assets.iter().find(|a| a.name() == "palettes").unwrap();
        assert_eq!(
            palettes(&map, pal, dir.path()).unwrap(),
            [[0x0f, 1, 2, 3], [0x0f, 0x11, 0x12, 0x13]]
        );
    }
}
