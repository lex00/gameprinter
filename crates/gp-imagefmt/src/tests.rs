//! Sheets out and back in, on made-up sprites.

use std::collections::BTreeMap;

use crate::config::{
    self, Color, Editor, ImageConfig, Layer, Layout, Order, PaletteFile, Transparency,
};
use crate::sheet::{self, Frame, Tile};

fn palette() -> [[u8; 3]; 16] {
    std::array::from_fn(|i| [i as u8 * 15, 250 - i as u8 * 15, i as u8 * 7])
}

/// A 16x8 frame of two tiles: the left in palette 0, the right in 1.
fn frame(name: &str, state: &str, shade: u8) -> Frame {
    let mut px = vec![0u8; 16 * 8];
    for y in 0..8 {
        for x in 0..8 {
            if (x + y) % 2 == 0 {
                px[y * 16 + x] = 1 + shade % 3;
            }
            if x == y {
                px[y * 16 + 8 + x] = 4 + 2;
            }
        }
    }
    Frame {
        name: name.into(),
        entity: "blob".into(),
        state: state.into(),
        set_index: 0,
        width: 16,
        height: 8,
        origin: [-8, -8],
        tiles: vec![
            Tile {
                x: 0,
                y: 0,
                palette: 0,
                flip_x: false,
                flip_y: false,
            },
            Tile {
                x: 8,
                y: 0,
                palette: 1,
                flip_x: false,
                flip_y: false,
            },
        ],
        px,
    }
}

fn files(out: Vec<(String, Vec<u8>)>) -> BTreeMap<String, Vec<u8>> {
    out.into_iter().collect()
}

#[test]
fn layers_resolve_by_template_and_slot() {
    let home = Layer::parse("color = \"rgba\"\nscale = 2\n").unwrap();
    let project = Layer::parse(
        "[image]\nlayout = \"frames\"\n[image.templates.game]\nscale = 3\n[image.templates.game.slots.hero]\npalette_file = \"hex\"\n",
    )
    .unwrap();
    let c = config::resolve(&[home.clone(), project.clone()], Some("game"), Some("hero")).unwrap();
    assert_eq!(
        (c.color, c.layout, c.scale, c.palette_file),
        (Color::Rgba, Layout::Frames, 3, PaletteFile::Hex)
    );
    let c = config::resolve(&[home, project], Some("other"), None).unwrap();
    assert_eq!((c.scale, c.palette_file), (2, PaletteFile::Gpl));
    assert!(Layer::parse("format = \"0.9.0\"").is_err());
    assert!(Layer::parse("colour = \"rgba\"").is_err());
    let bad = Layer::parse("color = \"indexed\"\ntransparency = \"key\"").unwrap();
    assert!(config::resolve(&[bad], None, None).is_err());
    assert!(config::schema().contains("game-printer:image-config@0.1.0"));
}

#[test]
fn every_layout_round_trips_exactly() {
    let frames = vec![
        frame("blob_walk_0", "walk", 0),
        frame("blob_walk_1", "walk", 1),
        frame("blob_hurt_2", "hurt", 2),
    ];
    for (layout, color, order, scale, transparency) in [
        (
            Layout::Sheet,
            Color::Indexed,
            Order::StateFrame,
            1,
            Transparency::Alpha,
        ),
        (
            Layout::Sheet,
            Color::Rgba,
            Order::Frame,
            2,
            Transparency::Alpha,
        ),
        (
            Layout::Frames,
            Color::Rgba,
            Order::Frame,
            1,
            Transparency::Key,
        ),
        (
            Layout::Frames,
            Color::Indexed,
            Order::Frame,
            3,
            Transparency::Alpha,
        ),
    ] {
        let cfg = ImageConfig {
            layout,
            color,
            order,
            scale,
            transparency,
            columns: 2,
            padding: 1,
            editor: Editor::Aseprite,
            palette_file: PaletteFile::Pixelorama,
            ..ImageConfig::default()
        };
        let out = files(sheet::write(&cfg, "blob", &frames, &palette()));
        assert!(out.contains_key("sprites.json") && out.contains_key("palette.json"));
        assert!(out.contains_key("sprites.aseprite.json"));
        let back = sheet::read(&|n| out.get(n).cloned(), None).unwrap();
        assert!(
            back.problems.is_empty(),
            "{layout:?} {color:?}: {:?}",
            back.problems
        );
        assert_eq!(back.frames, frames, "{layout:?} {color:?} {order:?}");
        assert_eq!(back.tiles, 4);
    }
}

#[test]
fn an_import_is_checked_against_its_tiles() {
    let frames = vec![frame("blob_walk_0", "walk", 0)];
    let cfg = ImageConfig {
        layout: Layout::Frames,
        color: Color::Indexed,
        ..ImageConfig::default()
    };
    let mut out = files(sheet::write(&cfg, "blob", &frames, &palette()));
    // Edit the image: a palette 1 pixel in the left tile, and a pixel
    // outside both tiles (made by widening the image), a stray index.
    let (w, h, px) = match sheet::read_png(&out["blob_walk_0.png"]).unwrap() {
        (w, h, sheet::Pixels::Indexed(px)) => (w, h, px),
        _ => unreachable!(),
    };
    let mut px = px;
    px[(3 * w + 1) as usize] = 5;
    px[(7 * w + 15) as usize] = 1;
    let mut edited = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut edited, w, h);
        enc.set_color(png::ColorType::Indexed);
        enc.set_palette(palette().as_flattened().to_vec());
        let mut wr = enc.write_header().unwrap();
        wr.write_image_data(&px).unwrap();
    }
    out.insert("blob_walk_0.png".into(), edited);
    let back = sheet::read(&|n| out.get(n).cloned(), None).unwrap();
    let msgs: Vec<&str> = back.problems.iter().map(|p| p.message.as_str()).collect();
    assert!(
        msgs.iter().any(|m| m.contains("mixes palettes")),
        "{msgs:?}"
    );
    assert!(
        msgs.iter()
            .any(|m| m.contains("palette 0, the tile's is 1")),
        "{msgs:?}"
    );
    // A budget of one tile is too small.
    let back = sheet::read(&|n| out.get(n).cloned(), Some(1)).unwrap();
    assert!(back.problems.iter().any(|p| p.message.contains("budget")));
}

#[test]
fn a_tile_and_its_mirror_count_once() {
    let mut f = frame("blob_x_0", "x", 0);
    // The right tile becomes the left tile mirrored, in its own palette.
    for y in 0..8 {
        for x in 0..8 {
            let left = f.px[y * 16 + x];
            f.px[y * 16 + 8 + (7 - x)] = if left == 0 { 0 } else { left + 4 };
        }
    }
    let cfg = ImageConfig::default();
    let out = files(sheet::write(&cfg, "blob", &[f], &palette()));
    let back = sheet::read(&|n| out.get(n).cloned(), None).unwrap();
    assert_eq!((back.tiles, back.saved_by_flips), (1, 1));
}

#[test]
fn the_schema_file_is_current() {
    let file = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../schema/image-config.json"
    );
    let have = std::fs::read_to_string(file).unwrap_or_default();
    assert_eq!(
        have,
        config::schema(),
        "run: gp sprites schema > schema/image-config.json"
    );
}
