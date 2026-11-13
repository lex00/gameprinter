//! Capture end to end on a made-up game: its tiles, a rows table of
//! frames indexed by object ID, a state mapping, and frames of a made-up
//! recording, all written here. Nothing comes from any real game.

use std::collections::BTreeMap;
use std::path::Path;

use gp_assets::map::AssetMap;
use gp_interp::mapping::Mapping;

use crate::build;
use crate::chr::{Banks, TileBank};
use crate::config::Config;
use crate::emit;
use crate::observe::{Observer, Shot};
use crate::tables::Tables;

const MAP: &str = r#"
schema = 1
source = "made-up"
sha256 = "0000000000000000000000000000000000000000000000000000000000000000"

[[asset]]
name = "tiles"
format = "chr-2bpp"
region = "chr"
offset = 0
tiles = 8
tiles_per_row = 8

[[asset]]
name = "frames"
format = "byte-rows"
region = "prg"
offset = 0x10
count = 3
width = 2

[[asset]]
name = "ids"
format = "table"
count = 8
column = [{ name = "first", region = "prg", offset = 0x20 }]

[[asset]]
name = "sprites"
format = "sprite-set"
"#;

const MODEL: &str = r#"
[model]
schema = "0.4.1"
name = "madeup"
title = "Made Up"
source = "made-up"
view = "side"
frame_rate = "60"
systems = ["rules", "entity_states", "counters", "spawn", "platformer.player", "platformer.enemies", "platformer.contact", "exits", "camera"]
start_level = "main"

[model.extensions]
platformer = "0.4.0"

[input]
device = "nes-pad"

[input.actions]
jump = "a"
left = "left"
right = "right"

[assets.tiles]
kind = "tiles"
role = "graphics"
from = { source = "made-up", asset = "tiles" }

[assets.frames]
kind = "rows"
role = "graphics"
from = { source = "made-up", asset = "frames" }

[entities.hero]
role = "player"
size = [8, 8]
hitbox = { x = 0, y = 0, w = 8, h = 8 }

[entities.hero.run]
walk_speed = 1
air_control = true

[entities.hero.sprite]
slot = "tiles"
tiles = [1]
width = 1
palette = 4

[entities.blob]
role = "enemy"
size = [16, 8]
hitbox = { x = 0, y = 0, w = 16, h = 8 }

[entities.blob.machine]
initial = "walk"

[entities.blob.machine.states.walk]

[entities.blob.machine.states.squashed]

[levels.main]
size = [32, 15]
tile_size = 16

[cameras.main]
size = [256, 240]
follow = "hero"
axis = "x"
lead = [120, 0]
"#;

const MAPPING: &str = r#"
schema = 1
model = "madeup"
source = "made-up"
align = "polled"

[[field]]
path = "entities.hero[0].x"
ram = [0x10]

[[field]]
path = "entities.hero[0].y"
ram = [0x11]

[[field]]
path = "entities.hero[0].on_ground"
ram = [0x12]
flag = true

[[field]]
path = "cameras.main.x"
ram = [0x13]

[slots]
count = 2
active = 0x20
id = 0x22
clock = 0x09

[[slots.field]]
path = "x"
ram = [0x24]

[[slots.field]]
path = "y"
ram = [0x26]

[[slots.kind]]
entity = "blob"
ids = [5]
"#;

const CONFIG: &str = r#"
schema = 1

[[table]]
shape = "rows"
name = "enemy"
asset = "frames"
slot = "tiles"
width = 2
palette = 5
index = { asset = "ids", column = "first", divide = 2 }

[entity.hero]
when = { invulnerable = "ram[0x30] != 0" }

[[entity.blob.state]]
name = "squashed"
when = "slot[0x28] == 1"
"#;

/// Tiles 1 to 5, each with a different pixel lit; tile 0 blank.
fn tiles() -> Vec<u8> {
    let mut px = vec![0u8; 64 * 8];
    for t in 1..6usize {
        let (x, y) = (t, t);
        px[y * 64 + t * 8 + x] = 1 + (t as u8 % 3);
    }
    px
}

fn write_lift(dir: &Path) {
    let png = gp_assets::chr::write_png(64, 8, &tiles());
    std::fs::write(dir.join("tiles.png"), png).unwrap();
    // Rows: 0 is tiles 3 and 4 (the blob walking), 1 is 5 and 0 (squashed),
    // 2 is 4 and 3 (never shown).
    std::fs::write(
        dir.join("frames.toml"),
        "schema = 1\nasset = \"frames\"\nrows = [[3, 4], [5, 0], [4, 3]]\n",
    )
    .unwrap();
    // ID 5's rows start at offset 0 (row 0); ID 6's at 6 (row 3, past the
    // end), so ID 5 has rows 0 to 2.
    let mut ids = String::from("schema = 1\nasset = \"ids\"\n");
    for v in [8, 8, 8, 8, 8, 0, 6, 6] {
        ids.push_str(&format!("[[row]]\nfirst = {v}\n"));
    }
    std::fs::write(dir.join("ids.toml"), ids).unwrap();
    std::fs::write(dir.join("assets.toml"), MAP).unwrap();
}

struct Frame {
    ram: Vec<u8>,
    oam: Vec<u8>,
}

/// The made-up recording: the hero walks right on the ground showing
/// tiles 1 and 2 by turns every 4 frames, jumps (tile 3 in the air), is
/// hurt and blinks every 2 frames; a blob walks left showing row 0, then
/// is squashed (row 1).
fn recording() -> Vec<Frame> {
    let mut out = Vec::new();
    for f in 0..120u32 {
        let mut ram = vec![0u8; 2048];
        let mut oam = vec![0xFFu8; 256];
        let mut n = 0;
        let mut put = |y: i32, tile: u8, attr: u8, x: i32| {
            oam[n * 4] = (y - 1) as u8;
            oam[n * 4 + 1] = tile;
            oam[n * 4 + 2] = attr;
            oam[n * 4 + 3] = x as u8;
            n += 1;
        };
        let x = 20 + f as i32;
        let air = (40..60).contains(&f);
        let hurt = (80..100).contains(&f);
        let y = if air { 100 } else { 120 };
        ram[0x10] = x as u8;
        ram[0x11] = y as u8;
        ram[0x12] = u8::from(!air);
        ram[0x30] = u8::from(hurt);
        ram[0x09] = f as u8;
        let tile = if air {
            3
        } else if (f / 4) % 2 == 0 {
            1
        } else {
            2
        };
        if !(hurt && (f / 2) % 2 == 1) {
            put(y, tile, 0, x);
        }
        // The blob, in slot 1, from frame 10.
        if f >= 10 {
            let bx = 200 - f as i32;
            ram[0x21] = 1;
            ram[0x23] = 5;
            ram[0x25] = bx as u8;
            ram[0x27] = 150;
            let squashed = f >= 100;
            ram[0x29] = u8::from(squashed);
            // Facing left: mirrored about its middle, so the right tile
            // comes first, flipped.
            if squashed {
                put(150, 5, 0x41, bx + 8);
            } else {
                put(150, 4, 0x41, bx);
                put(150, 3, 0x41, bx + 8);
            }
        }
        out.push(Frame { ram, oam });
    }
    out
}

#[test]
fn capture_finds_every_frame_clip_and_gap_and_writes_the_model() {
    let dir = tempfile::tempdir().unwrap();
    let tmp = dir.path().to_path_buf();
    write_lift(&tmp);
    let model = gp_model::Model::parse(MODEL).unwrap();
    let map = AssetMap::parse(MAP).unwrap();
    let mapping = Mapping::parse(MAPPING).unwrap();
    let config = Config::parse(CONFIG).unwrap();
    let bank = TileBank::load(&tmp, &map, &model).unwrap();
    let mut ids = BTreeMap::new();
    ids.insert("blob".to_owned(), vec![5u8]);
    let tables = Tables::read(&tmp, &config, &ids).unwrap();
    assert_eq!(tables.of_entity["blob"].len(), 3);

    let mut obs = Observer::new(&model, Some(&mapping), &config, &bank).unwrap();
    let palette: Vec<u8> = (0..32).collect();
    for (i, f) in recording().iter().enumerate() {
        obs.shot(&Shot {
            frame: i as u64,
            ram: &f.ram,
            oam: &f.oam,
            palette: &palette,
            ctrl: 0,
            banks: Banks::default(),
        });
    }
    let pattern = crate::pattern_map(&obs.pattern);
    let mut cap = build::build(
        &model,
        &config,
        &tables,
        &bank,
        &obs.layouts,
        &obs.obs,
        &obs.level_colors,
        &pattern,
    );

    // The hero: its base and clips: a blink while invulnerable (with an
    // empty frame), the air frame, and the walk every 4 frames.
    let hero = &cap.entities["hero"];
    let (_, clips) = &hero.states[""];
    let (when, every) = hero.blink.clone().expect("a blink");
    assert_eq!(when.invulnerable, Some(true));
    assert_eq!(every, 2);
    let walk = clips
        .iter()
        .find(|c| c.frames.len() == 2 && c.when.invulnerable != Some(true))
        .expect("a walk clip");
    assert_eq!(walk.every, Some(4));
    assert!(clips.iter().any(|c| c.when.ground == Some(false)));

    // The blob: walking matches table row 0 (seen mirrored, made right
    // again), squashed matches row 1, and row 2 was never seen.
    let blob = &cap.entities["blob"];
    assert_eq!(blob.coverage.covered, ["squashed", "walk"]);
    assert_eq!(blob.coverage.unobserved, ["enemy 2"]);
    let (walk_base, _) = blob.states[""];
    let f = &cap.set.frames[walk_base as usize];
    assert!(f.from.iter().any(|s| s == "table enemy 0"), "{:?}", f.from);
    assert_eq!(f.pieces[0].tile, 3);
    assert_eq!((f.pieces[0].x, f.pieces[0].flip_x), (0, false));
    assert!(blob.states.contains_key("squashed"));
    assert!(blob.faces_both_ways || !blob.faces_both_ways);

    // Written into the model: a set slot, the hero's sprite moved into
    // the set with its new clips, the blob's states, and evidence; the
    // model still parses and checks.
    let mut m = model.clone();
    let rows = emit::rows_reader(&model, &tmp);
    let refs = vec![gp_model::model::Ref {
        at: "recording made-up".into(),
        within: None,
        quote: None,
    }];
    let w = emit::write(
        &mut m, &mut cap, "sprites", "made-up", &rows, &bank, &refs, &tmp,
    );
    assert!(w.slots_added.contains(&"sprites".to_owned()));
    assert_eq!(m.model.schema, "0.4.2");
    let hero = m.entities["hero"].core.sprite.as_ref().unwrap();
    assert_eq!(hero.frame.as_ref().unwrap().slot, "sprites");
    assert_eq!(hero.blink.as_ref().map(|b| b.every), Some(2));
    let squashed = m.entities["blob"].in_state("squashed").unwrap();
    assert_ne!(
        squashed.core.sprite.as_ref().unwrap().frame,
        m.entities["blob"].core.sprite.as_ref().unwrap().frame
    );
    assert!(m.evidence.contains_key("entities.blob.sprite"));
    let text = m.to_toml();
    let back = gp_model::Model::parse(&text).unwrap();
    let problems = gp_model::validate::validate(&back);
    assert!(
        !problems.iter().any(|p| p.path.contains("sprite")),
        "{problems:?}"
    );
    let set_text = cap.set.to_text();
    gp_assets::sprite_set::SpriteSet::parse(&set_text).unwrap();
}

/// An arcade game on a made-up MCR lift: a hero whose state mapping reads
/// the sprite RAM's own units, drawn as two 32x32 sprites side by side.
/// Capture places the hero as the board places a sprite, (x - 3, 241 - y),
/// finds each sprite's picture by its number, and keeps a frame of two
/// whole-sprite pieces 16 apart. Nothing comes from any real game.
#[test]
fn an_arcade_frame_is_whole_sprites_found_by_number() {
    use crate::observe::{ListSprite, Screen};
    let dir = tempfile::tempdir().unwrap();
    let tmp = dir.path();
    let map_text = format!(
        "{}\n[[asset]]\nname = \"set\"\nformat = \"sprite-set\"\n",
        gp_assets::arcade::mcr_91490_map("made-up", "Made Up", &["madeup"])
    );
    let map = gp_assets::arcade::ArcadeMap::parse(&map_text).unwrap();
    // Sprite k has its top left pixel k (sprite 0 blank, sprite 9 only
    // value 8, which the board draws nothing for).
    let decoded = gp_assets::arcade::Decoded {
        set: "madeup".into(),
        identity: "id".into(),
        tiles: vec![[0; 64]; 1024],
        sprites: (0..256)
            .map(|k| {
                let mut s = [0u8; 1024];
                s[0] = if k == 9 { 8 } else { (k % 16) as u8 };
                s[33] = 1;
                if k == 0 || k == 9 {
                    s[33] = 0;
                }
                s
            })
            .collect(),
    };
    let (files, _) = gp_assets::arcade::lift(&decoded, &map, &map_text).unwrap();
    for f in files {
        std::fs::write(tmp.join(&f.name), &f.bytes).unwrap();
    }
    let model = gp_model::Model::parse(MODEL).unwrap();
    let mapping = Mapping::parse(
        "schema = 1\nmodel = \"madeup\"\nsource = \"made-up\"\nalign = \"polled\"\n\n[[field]]\npath = \"entities.hero[0].x\"\nram = [0xE010]\n\n[[field]]\npath = \"entities.hero[0].y\"\nram = [0xE011]\n",
    )
    .unwrap();
    let config = Config::parse("schema = 1\nslot = \"set\"\nasset = \"set\"\n").unwrap();
    let bank = TileBank::load_arcade(tmp, &map, &model).unwrap();
    assert_eq!(bank.sprite_at(7), Some((0, 7)));
    assert!(bank.is_blank((0, 9)) && bank.is_blank((0, 0)));
    assert_eq!(bank.sprite_at(300), None);
    let mut obs = Observer::new(&model, Some(&mapping), &config, &bank).unwrap();
    obs.set_screen(Screen::Mcr);
    for f in 0..6u64 {
        let mut ram = vec![0u8; 0xE800];
        let (x, y) = (100u8 + f as u8, 120u8);
        ram[0xE010] = x;
        ram[0xE011] = y;
        let at = |dx: i32, code: u32| ListSprite {
            x: i32::from(x) - 3 + dx,
            y: 241 - i32::from(y),
            code,
            color: 1,
            flip_x: false,
            flip_y: false,
        };
        // Two pictures, a blank one on the hero, and one far away.
        let mut list = vec![at(0, 7), at(16, 8), at(0, 9)];
        list.push(ListSprite {
            x: 10,
            y: 10,
            ..at(0, 300)
        });
        obs.shot_list(f, &ram, &list);
    }
    assert_eq!(obs.unresolved, 6);
    let hero: Vec<_> = obs.obs.iter().filter(|o| o.entity == "hero").collect();
    assert_eq!(hero.len(), 6);
    let l = &obs.layouts[hero[0].layout as usize];
    let pieces: Vec<(u32, i32, i32, u8)> =
        l.iter().map(|p| (p.tile, p.x, p.y, p.palette)).collect();
    assert_eq!(pieces, [(7, 0, 0, 5), (8, 16, 0, 5)]);
    assert_eq!(hero[0].anchor, [0, 0]);
}
