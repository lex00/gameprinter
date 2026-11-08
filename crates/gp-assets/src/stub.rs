//! Placeholder assets ("stubs") in the lifted formats, with each asset's
//! shape and none of its content, made from the asset map alone (research
//! spike, docs/research/asset-stubs.md). A stub tiles file has the map's
//! tile count and layout, and each tile shows its own number; a stub
//! palettes file has the map's palette count, each palette one hue picked
//! by the role that draws with it. Neither reads a ROM or a lifted file, so
//! a template can be stubbed with no source at hand.

use std::fmt::Write as _;

use crate::chr;
use crate::map::SCHEMA;
use crate::text::hex_list;

/// 3 by 5 pixel hex digits, one row per entry, the leftmost pixel in bit 2.
const DIGITS: [[u8; 5]; 16] = [
    [0b111, 0b101, 0b101, 0b101, 0b111], // 0
    [0b010, 0b110, 0b010, 0b010, 0b111], // 1
    [0b111, 0b001, 0b111, 0b100, 0b111], // 2
    [0b111, 0b001, 0b011, 0b001, 0b111], // 3
    [0b101, 0b101, 0b111, 0b001, 0b001], // 4
    [0b111, 0b100, 0b111, 0b001, 0b111], // 5
    [0b111, 0b100, 0b111, 0b101, 0b111], // 6
    [0b111, 0b001, 0b010, 0b010, 0b010], // 7
    [0b111, 0b101, 0b111, 0b101, 0b111], // 8
    [0b111, 0b101, 0b111, 0b001, 0b111], // 9
    [0b111, 0b101, 0b111, 0b101, 0b101], // A
    [0b110, 0b101, 0b110, 0b101, 0b110], // B
    [0b111, 0b100, 0b100, 0b100, 0b111], // C
    [0b110, 0b101, 0b101, 0b101, 0b110], // D
    [0b111, 0b100, 0b110, 0b100, 0b111], // E
    [0b111, 0b100, 0b110, 0b100, 0b100], // F
];

/// One stub tile's color indices, row by row: a grid line along the top and
/// left edges (index 1 for tiles 0 to 255, index 2 for the next 256, so
/// the two pattern tables look different), and the low byte of the tile's
/// number in hex (index 3). Everything else is index 0, which sprites draw
/// as transparent.
pub fn tile_pixels(number: u32) -> [u8; 64] {
    let mut px = [0u8; 64];
    let grid = if number & 0x100 == 0 { 1 } else { 2 };
    for i in 0..8 {
        px[i] = grid;
        px[i * 8] = grid;
    }
    let byte = (number & 0xFF) as usize;
    for (digit, left) in [(byte >> 4, 1), (byte & 0xF, 5)] {
        for (row, bits) in DIGITS[digit].iter().enumerate() {
            for col in 0..3 {
                if bits >> (2 - col) & 1 == 1 {
                    px[(row + 2) * 8 + left + col] = 3;
                }
            }
        }
    }
    px
}

/// A stub tiles file: an indexed PNG of `tiles` stub tiles, `per_row` to a
/// row, the size and layout `gp assets lift` writes for that map entry.
pub fn tiles_png(tiles: u32, per_row: u32) -> Vec<u8> {
    let (w, h) = chr::image_size(tiles, per_row);
    let mut pixels = vec![0u8; (w * h) as usize];
    for t in 0..tiles {
        let (tx, ty) = ((t % per_row) * 8, (t / per_row) * 8);
        for (i, p) in tile_pixels(t).iter().enumerate() {
            let (x, y) = (i as u32 % 8, i as u32 / 8);
            pixels[((ty + y) * w + tx + x) as usize] = *p;
        }
    }
    chr::write_png(w, h, &pixels)
}

/// The NES hue (the low nibble of a color number) a role is drawn in.
/// Colors carry the role, so a stubbed game shows who is who.
pub fn role_hue(role: &str) -> u8 {
    match role {
        "player" => 0x0A,      // green
        "enemy" => 0x06,       // red
        "boss" => 0x05,        // crimson
        "projectile" => 0x07,  // orange
        "npc" => 0x0C,         // cyan
        "collectible" => 0x08, // yellow
        "power-up" => 0x09,    // lime
        "hazard" => 0x04,      // magenta
        "interactive" => 0x03, // purple
        "goal" => 0x0B,        // teal
        "sky" | "water" => 0x01,
        "terrain" | "platform" => 0x07,
        _ => 0x00, // gray
    }
}

/// One stub palette: black at index 0 (the backdrop, or transparent for
/// sprites), then the hue from dark to light. Gray when there is no role.
pub fn palette(role: Option<&str>) -> [u8; 4] {
    let hue = role.map_or(0x00, role_hue);
    [0x0F, hue, 0x10 | hue, 0x30 | hue]
}

/// A stub palettes file, one palette per entry of `roles` (the role whose
/// sprites or tiles use that palette, when one does), in the format
/// `gp assets lift` writes.
pub fn palettes_toml(asset: &str, roles: &[Option<&str>]) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "# Palettes: four NES color numbers each.\n\
         # A stub written by gp assets stub: one hue per palette, by the role that uses it."
    );
    let _ = writeln!(out, "schema = {SCHEMA}\nasset = \"{asset}\"");
    out.push_str("palettes = [\n");
    let count = roles.len();
    for (i, role) in roles.iter().enumerate() {
        let what = match (count, i) {
            (8, 0..4) => format!("background {i}"),
            (8, _) => format!("sprites {}", i - 4),
            _ => i.to_string(),
        };
        let note = role.map_or(String::new(), |r| format!(": {r}"));
        let _ = writeln!(out, "  {}, # {what}{note}", hex_list(&palette(*role)));
    }
    out.push_str("]\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tile_shows_its_number() {
        let px = tile_pixels(0x1A);
        // The grid: the top row and left column.
        assert!(px[..8].iter().all(|&p| p == 1));
        assert!((0..8).all(|r| px[r * 8] == 1));
        // "1" has its middle column set on every row, "A" its left column.
        assert!((2..7).all(|r| px[r * 8 + 2] == 3));
        assert!((2..7).all(|r| px[r * 8 + 5] == 3));
        // The second pattern table's tiles have the other grid color.
        let other = tile_pixels(0x11A);
        assert_eq!(other[0], 2);
        assert!((1..8).all(|r| other[r * 8 + 1..r * 8 + 8] == px[r * 8 + 1..r * 8 + 8]));
    }

    #[test]
    fn stub_tiles_pack_like_lifted_ones() {
        let png = tiles_png(20, 16);
        let (w, h) = chr::image_size(20, 16);
        let pixels = chr::read_png(&png, (w, h)).unwrap();
        let bytes = chr::encode(&pixels, w, 20, 16);
        assert_eq!(bytes.len(), 20 * 16);
        let (_, _, back) = chr::decode(&bytes, 16);
        // Past the last tile the image stays index 0, as pack requires.
        assert_eq!(&back[..], &pixels[..back.len()]);
        assert_eq!(tiles_png(20, 16), png, "the same bytes every run");
    }

    #[test]
    fn no_asset_may_be_named_like_the_state_file() {
        let map = |name: &str| {
            format!(
                "schema = 1\nsource = \"x\"\nsha256 = \"{}\"\n[[asset]]\nname = \"{name}\"\nformat = \"palette\"\nregion = \"prg\"\noffset = 0\ncount = 1\n",
                "0".repeat(64)
            )
        };
        assert!(crate::AssetMap::parse(&map("pals")).is_ok());
        let e = crate::AssetMap::parse(&map("slots"))
            .unwrap_err()
            .to_string();
        assert!(e.contains("slots.toml"), "{e}");
    }

    #[test]
    fn palettes_by_role() {
        assert_eq!(palette(Some("player")), [0x0F, 0x0A, 0x1A, 0x3A]);
        assert_eq!(palette(None), [0x0F, 0x00, 0x10, 0x30]);
        let text = palettes_toml("palettes", &[None, Some("enemy")]);
        assert!(text.contains("[0x0F, 0x06, 0x16, 0x36], # 1: enemy"));
    }
}
