//! A made-up game for tests of this crate and the crates built on it: a
//! model with a 20-tile sheet (a 128 by 16 image with room past the last
//! tile), a palettes slot, a second tiles slot on the same asset, one
//! baseline and a stand-in for `gp` whose model check can be made to fail
//! (by a file named `fail` beside it). It holds no game data.

use std::fs;
use std::path::{Path, PathBuf};

use crate::Config;

pub const MODEL: &str = r#"
[model]
schema = "0.4.0"
name = "tiny"
title = "Tiny"
source = "tiny"
start_level = "main"
view = "side"
frame_rate = "60"
systems = ["rules", "entity_states", "counters", "spawn", "platformer.player", "platformer.enemies", "platformer.contact", "exits", "camera"]

[model.extensions]
platformer = "0.4.0"

[input]
device = "nes-pad"

[input.actions]
jump = "a"
left = "left"
right = "right"
run = "b"

[assets.level]
kind = "level-grid"
role = "layout"

[assets.level.from]
source = "tiny"
asset = "level"

[assets.metatiles]
kind = "metatiles"
role = "tileset"

[assets.metatiles.from]
source = "tiny"
asset = "metatiles"

[assets.tiles]
kind = "tiles"
role = "graphics"

[assets.tiles.from]
source = "tiny"
asset = "tiles"

[assets.tiles_again]
kind = "tiles"
role = "graphics"

[assets.tiles_again.from]
source = "tiny"
asset = "tiles"

[assets.palettes]
kind = "palettes"
role = "palettes"

[assets.palettes.from]
source = "tiny"
asset = "palettes"

[entities.hero]
role = "player"
size = [16, 16]

[entities.hero.run]
air_control = true
run_action = "run"
run_speed = 2
walk_speed = 1

[entities.hero.jump]
action = "jump"
gravity = "0.25"
max_fall = 4
speed = -5

[entities.hero.hitbox]
x = 0
y = 0
w = 16
h = 16

[entities.hero.sprite]
palette = 4
slot = "tiles"
tiles = [16, 17, 18, 19]
width = 2

[entities.buddy]
role = "npc"
size = [16, 16]

[entities.buddy.hitbox]
x = 0
y = 0
w = 16
h = 16

[entities.buddy.sprite]
palette = 4
slot = "tiles"
tiles = [0, 1, 2, 3]
width = 2

[levels.main]
grid = "level"
kill_below = 64
size = [8, 4]
solid_property = "solid"
tile_size = 16
tileset = "metatiles"

[levels.main.goal]
x = 100

[[levels.main.spawn]]
entity = "hero"
x = 16
y = 16

[cameras.main]
follow = "hero"
axis = "x"
size = [256, 240]
lead = [120, 0]
clamp_to_level = true

[rules.game]
initial = "playing"

[rules.game.states.playing]
description = "Playing"
"#;

pub const MAP: &str = r#"
schema = 1
source = "tiny"
sha256 = "0000000000000000000000000000000000000000000000000000000000000001"

[[asset]]
name = "tiles"
format = "chr-2bpp"
region = "chr"
offset = 0
tiles = 20
tiles_per_row = 16

[[asset]]
name = "palettes"
format = "palette"
region = "prg"
offset = 0
count = 8

[[asset]]
name = "metatiles"
format = "metatiles"
region = "prg"
offset = 0x20
count = 2
names = ["sky", "ground"]
property = [
  { name = "solid", region = "prg", offset = 0x28 },
]

[[asset]]
name = "level"
format = "level-grid"
region = "prg"
offset = 0x30
width = 8
height = 4
legend = ".G"
metatiles = "metatiles"
"#;

pub const METATILES: &str = r#"schema = 1
asset = "metatiles"

[[metatile]]
tiles = [0, 0, 0, 0]
solid = 0

[[metatile]]
tiles = [1, 2, 3, 4]
solid = 1
"#;

pub const LEVEL: &str = r#"schema = 1
asset = "level"
rows = [
  "........",
  "........",
  "........",
  "GGGGGGGG",
]
"#;

pub const BASELINE: &str = r#"
schema = 0
source = "tiny"
sha256 = "0000000000000000000000000000000000000000000000000000000000000001"
frames = 3
holds = ["a@1-2"]
frame_hashes = "0000000000000000000000000000000000000000000000000000000000000002"
lag_frames = 0
"#;

/// A 128 by 16 sheet with `v` on the 20 tiles and 0 past them.
pub fn sheet(v: u8) -> Vec<u8> {
    let mut px = vec![0u8; 128 * 16];
    for t in 0..20usize {
        for y in 0..8 {
            for x in 0..8 {
                px[((t / 16) * 8 + y) * 128 + (t % 16) * 8 + x] = v;
            }
        }
    }
    px
}

/// Lays the game out under `root` (`repo/`, `lifted/`, `work/copy/`) and
/// returns a configuration for it and the stand-in `gp`'s path.
pub fn config(root: &Path) -> (Config, PathBuf) {
    let repo = root.join("repo");
    let lifted = root.join("lifted");
    let work = root.join("work");
    for d in [
        repo.join("baselines/tiny"),
        repo.join("templates"),
        lifted.clone(),
        work.join("copy"),
    ] {
        fs::create_dir_all(d).unwrap();
    }
    fs::write(repo.join("baselines/tiny/one.toml"), BASELINE).unwrap();
    fs::write(lifted.join("assets.toml"), MAP).unwrap();
    fs::write(lifted.join("metatiles.toml"), METATILES).unwrap();
    fs::write(lifted.join("level.toml"), LEVEL).unwrap();
    fs::write(
        lifted.join("tiles.png"),
        gp_assets::chr::write_png(128, 16, &sheet(1)),
    )
    .unwrap();
    let pal = vec![[0x0F, 0x01, 0x11, 0x21]; 8];
    fs::write(
        lifted.join("palettes.toml"),
        crate::place::write_palettes("palettes", &pal, "the test"),
    )
    .unwrap();
    fs::write(work.join("copy/model.toml"), MODEL).unwrap();
    let gp = root.join("gp");
    fs::write(
        &gp,
        "#!/bin/sh\nif [ -e \"$(dirname \"$0\")/fail\" ]; then echo '{\"problems\":[\"made to fail\"]}'; exit 1; fi\necho '{\"ok\":true}'\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&gp, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let config = Config {
        gp: gp.clone(),
        repo,
        work,
        home: root.join("home"),
        rom: None,
        assets: Some(lifted),
        games: None,
    };
    (config, gp)
}
