//! `gp game new NAME --empty`: a new game from nothing, not from a
//! template. It writes the smallest game that builds, validates and plays
//! on the page, as a person's copy in `$GP_HOME/work/mcp/NAME/` (where
//! `template_fork` puts a copy of a template), so every tool that grows a
//! copy takes it as it is: `gp level edit` and `gp level verify`, `gp theme
//! look new`, `model_edit` for a new element, `build_publish` and `gp game
//! share`.
//!
//! | Written | What |
//! |---|---|
//! | `model.toml` | One stage (`main`), one player that walks, runs and jumps, a camera, and rules: reaching the goal wins, falling out starts the stage again. No enemies, no counters, no sound |
//! | `authored.toml` | The assets as data (docs/originals.md): the tiles from `tiles.pix`, eight palettes, four metatiles (air, ground, the goal's flag, earth) and the stage, 16 by 15 cells of floor |
//! | `tiles.pix` | The pictures, drawn here as plain shapes: a block of ground, a square with two eyes for the player, a flag for the goal |
//! | `template.toml` | The manifest: an original (`source.how = "authored"`), its pins set by the build |
//! | `asset-map.toml` | The asset map `gp template build` writes from `authored.toml` |
//! | `fidelity.toml`, `profiles.toml` | The contract (held to its own model, the look free) and the profiles (no switches) |
//! | `level-kit/` | The level kit's rules (what of the stage is data, what the model fixes, the limits) and the kit `gp level kit` writes from them |
//! | `themes/kit/kit.toml` | The theme kit `gp theme kit` writes: the player's picture to draw, so `gp theme look new` starts with it |
//! | `AGENTS.md` | What each file is and how to grow the game |
//!
//! Then it builds the assets (`gp template build --write`, into
//! `$GP_HOME/work/assets/NAME`) and writes the new-game journey's record
//! (`$GP_HOME/work/journeys/new/NAME/journey.json`, from_kind `empty`), so
//! the page and `next` show the copy as a game started from nothing.
//! Nothing is written over: a copy or a lift of that name is refused.

use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context as _, Result, bail};
use gp_evidence::Paths;
use serde_json::json;

use crate::Outcome;
use crate::journeys::{self, Kind};

/// What `gp game new NAME --empty` was asked.
pub struct Empty<'a> {
    pub name: &'a str,
    pub title: Option<&'a str>,
}

/// The stage's size in cells, and the cells' size in pixels.
const WIDTH: usize = 16;
const HEIGHT: usize = 15;
const CELL: usize = 16;
/// The rows of floor at the bottom of the stage.
const FLOOR_ROWS: usize = 3;
/// The goal's column, and the player's start in pixels.
const GOAL_COLUMN: usize = 13;
const START: (usize, usize) = (24, 176);

/// The copy's folder for a name.
pub(crate) fn copy_dir(paths: &Paths, name: &str) -> PathBuf {
    journeys::copies_root(&paths.home).join(name)
}

/// A title from a name: `my_game` reads `My game`.
fn title_of(name: &str) -> String {
    let words = name.replace('_', " ");
    let mut c = words.chars();
    c.next()
        .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
        .unwrap_or_default()
}

/// A TOML string, quoted.
fn q(s: &str) -> String {
    toml::Value::String(s.to_owned()).to_string()
}

pub fn run(a: &Empty<'_>, paths: &Paths) -> Result<Outcome> {
    let name = a.name;
    if !gp_model::path::is_name(name) {
        bail!(
            "a new game's name is letters, digits and _, starting with a letter (like my_hop): {name}"
        );
    }
    let dir = copy_dir(paths, name);
    if dir.exists() {
        bail!(
            "{} exists already: gp game new writes over nothing; pick another name",
            dir.display()
        );
    }
    let lift = paths.home.join("work").join("assets").join(name);
    if lift.exists() {
        bail!(
            "{} exists already (another game's pictures and levels by that name): pick another name",
            lift.display()
        );
    }
    let title = a.title.map(str::to_owned).unwrap_or_else(|| title_of(name));
    let files = files(name, &title);
    // Written under another name and renamed into place, so a start that
    // stopped halfway is never taken for a game.
    let root = journeys::copies_root(&paths.home);
    fs::create_dir_all(&root).with_context(|| format!("creating {}", root.display()))?;
    let tmp = root.join(format!(".{name}.tmp-{}", std::process::id()));
    for (rel, text) in &files {
        let p = tmp.join(rel);
        if let Some(d) = p.parent() {
            fs::create_dir_all(d).with_context(|| format!("creating {}", d.display()))?;
        }
        fs::write(&p, text).with_context(|| format!("writing {}", p.display()))?;
    }
    fs::rename(&tmp, &dir).with_context(|| format!("renaming {} into place", tmp.display()))?;

    // The assets and the pins, then the level kit from them.
    let built = crate::template_build::build_cmd(&dir, None, true, paths)?;
    let (m, _) = crate::theme::read_model(&dir)?;
    let ldir = dir.join(crate::level_kit::DIR);
    crate::level_kit::write(&dir, &m, &ldir)?;
    // The theme kit, so a theme of it starts with the player to draw.
    let (kit, _) = crate::theme_kit::kit_for(&m, name, &dir)?;
    let tdir = dir.join("themes").join("kit");
    fs::create_dir_all(&tdir).with_context(|| format!("creating {}", tdir.display()))?;
    fs::write(
        tdir.join(crate::theme_kit::KIT_FILE),
        crate::theme_kit::kit_text(&kit),
    )
    .context("writing the theme kit")?;
    journeys::write(
        &paths.home,
        Kind::New,
        name,
        "journey.json",
        &json!({"journey": "new", "name": name, "from": "nothing", "from_kind": "empty",
                "made_ms": journeys::now_ms(), "steps_done": []}),
    )?;

    let mut wrote: Vec<String> = files.iter().map(|(f, _)| f.clone()).collect();
    wrote.push("asset-map.toml".into());
    wrote.push(format!(
        "{}/{}",
        crate::level_kit::DIR,
        crate::level_kit::KIT_FILE
    ));
    wrote.push(format!(
        "{}/{}",
        crate::level_kit::DIR,
        crate::level_kit::DOC_FILE
    ));
    wrote.push(format!("themes/kit/{}", crate::theme_kit::KIT_FILE));
    wrote.sort();
    let mut text = vec![
        format!("{name}: a new game from nothing, in {}", dir.display()),
        format!(
            "  stage    main, {WIDTH} by {HEIGHT} cells of {CELL} px: a floor, the start at the left, the goal at the right"
        ),
        "  player   a square that walks (left, right), runs (B) and jumps (A)".into(),
        "  pictures its own, drawn as plain shapes in tiles.pix; no sound".into(),
        format!(
            "  assets   built from authored.toml into {}",
            built.json["assets"].as_str().unwrap_or_default()
        ),
        String::new(),
        "Wrote:".into(),
    ];
    text.extend(wrote.iter().map(|f| format!("  {name}/{f}")));
    text.extend([
        String::new(),
        format!(
            "Next: gp validate {name}; build it to play (MCP build_publish, or the page's BUILD IT TO PLAY); change the stage with gp level edit {name} --op \"set row level ROW V,V,..\"; give it a look with gp theme look new; add an element with model_edit."
        ),
    ]);
    Ok(Outcome::ok(
        json!({
            "game": name, "title": title, "dir": dir, "from": "nothing",
            "wrote": wrote, "assets": built.json["assets"],
            "level": "main", "size": [WIDTH, HEIGHT],
        }),
        text.join("\n"),
    ))
}

/// Every file the new game starts with, by its path in the folder.
fn files(name: &str, title: &str) -> Vec<(String, String)> {
    vec![
        ("model.toml".into(), model_toml(name, title)),
        ("authored.toml".into(), authored_toml(name, title)),
        ("tiles.pix".into(), tiles_pix()),
        ("template.toml".into(), template_toml(name, title)),
        ("fidelity.toml".into(), fidelity_toml(name, title)),
        ("profiles.toml".into(), profiles_toml(title)),
        (
            format!("{}/{}", crate::level_kit::DIR, crate::level_kit::RULES_FILE),
            rules_toml(title),
        ),
        ("AGENTS.md".into(), agents_md(name, title)),
    ]
}

/// The goal's x in pixels: the left edge of its column.
fn goal_x() -> usize {
    GOAL_COLUMN * CELL
}

fn model_toml(name: &str, title: &str) -> String {
    let mut s = format!(
        r#"# {title}: a new game started from nothing (gp game new --empty). Every
# fact here is the person's own, stated, for them to change.
[model]
schema = "0.4.0"
name = {name_q}
title = {title_q}
description = "A new game started from nothing: one stage of floor, a player that walks, runs and jumps, and a goal to reach"
source = {name_q}
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
run = "b"

[assets.level]
kind = "level-grid"
role = "layout"
description = "The stage: one character a cell, from the metatiles' legend"

[assets.level.from]
source = {name_q}
asset = "level"

[assets.metatiles]
kind = "metatiles"
role = "tileset"
description = "Air, ground with grass on top, the goal's flag, and earth"
item_roles = ["background", "terrain", "goal", "terrain"]

[assets.metatiles.from]
source = {name_q}
asset = "metatiles"

[assets.palettes]
kind = "palettes"
role = "palettes"
item_roles = ["terrain", "terrain", "goal", "background", "player", "enemy", "hazard", "collectible"]

[assets.palettes.from]
source = {name_q}
asset = "palettes"

[assets.tiles]
kind = "tiles"
role = "graphics"
description = "Plain shapes drawn in tiles.pix"

[assets.tiles.from]
source = {name_q}
asset = "tiles"

[entities.player]
description = "The player: a plain square that walks, runs and jumps"
role = "player"
size = [16, 16]
tile_collision = true

[entities.player.hitbox]
h = 16
w = 12
x = 2
y = 0

[entities.player.jump]
action = "jump"
gravity = "0.25"
max_fall = 4
speed = -5

[entities.player.run]
air_control = true
run_action = "run"
run_speed = 2
walk_speed = 1

[entities.player.sprite]
flip_when_facing_left = true
palette = 4
slot = "tiles"
tiles = [2, 3, 18, 19]
width = 2

[levels.main]
description = "One screen of open floor, the start at the left and the goal at the right"
grid = "level"
kill_below = 208
size = [{WIDTH}, {HEIGHT}]
solid_property = "solid"
tile_size = {CELL}
tileset = "metatiles"

[levels.main.goal]
x = {goal_x}

[[levels.main.spawn]]
entity = "player"
x = {sx}
y = {sy}

[cameras.main]
follow = "player"
axis = "x"
size = [256, 240]
lead = [112, 0]
clamp_to_level = true

[rules.game]
initial = "playing"

[[rules.game.states.playing.transition]]
to = "won"
on = "goal.reached"

[[rules.game.states.playing.transition]]
to = "dying"
on = "player.died"

[rules.game.states.dying]
description = "The stage waits a moment, then starts again"
freeze = true

[[rules.game.states.dying.transition]]
to = "playing"
after = 60
do = ["restart-level"]

[rules.game.states.won]
description = "The goal is reached: the game is won"
freeze = true
"#,
        name_q = q(name),
        title_q = q(title),
        goal_x = goal_x(),
        sx = START.0,
        sy = START.1,
    );
    for part in ["assets", "cameras", "entities", "input", "levels", "rules"] {
        let _ = write!(
            s,
            "\n[evidence.{part}]\nsource = \"stated\"\nconfidence = 100\nnote = \"Written by gp game new --empty, for the person to change\"\n\n[[evidence.{part}.refs]]\nat = {}\n",
            q(&format!("template:{name}"))
        );
    }
    s
}

/// The stage's rows: air, the goal's flag standing on the floor, and the
/// floor.
fn level_rows() -> Vec<String> {
    (0..HEIGHT)
        .map(|y| {
            if y == HEIGHT - FLOOR_ROWS {
                "#".repeat(WIDTH)
            } else if y > HEIGHT - FLOOR_ROWS {
                "=".repeat(WIDTH)
            } else if y == HEIGHT - FLOOR_ROWS - 1 {
                (0..WIDTH)
                    .map(|x| if x == GOAL_COLUMN { 'G' } else { '.' })
                    .collect()
            } else {
                ".".repeat(WIDTH)
            }
        })
        .collect()
}

fn authored_toml(name: &str, title: &str) -> String {
    let rows: Vec<String> = level_rows()
        .iter()
        .map(|r| format!("  \"{r}\",\n"))
        .collect();
    format!(
        r##"# {title}: the assets, as data (docs/originals.md). `gp template build`
# lays them out and writes asset-map.toml; levels are one character per
# 16x16 cell, from the metatiles' chars.
format = "0.1.0"
source = {name_q}
title = {title_q}

# The pictures: 32 tiles of 8x8, 16 a row, drawn in tiles.pix.
[[asset]]
name = "tiles"
format = "chr-2bpp"
tiles = 32
tiles_per_row = 16
file = "tiles.pix"

# Four background palettes, then four sprite palettes (the player, then
# three spare). NES color numbers. The stage is drawn in the first: sky
# blue, brown earth, green grass on top, dark earth.
[[asset]]
name = "palettes"
format = "palette"
palettes = [
  [0x21, 0x17, 0x2A, 0x07],
  [0x21, 0x17, 0x2A, 0x07],
  [0x21, 0x28, 0x16, 0x0F],
  [0x21, 0x30, 0x10, 0x00],
  [0x21, 0x16, 0x30, 0x0F],
  [0x21, 0x12, 0x30, 0x0F],
  [0x21, 0x06, 0x26, 0x0F],
  [0x21, 0x28, 0x38, 0x0F],
]

[[asset]]
name = "metatiles"
format = "metatiles"
properties = ["solid"]
metatile = [
  {{ name = "air", char = ".", tiles = [0x00, 0x00, 0x00, 0x00], solid = 0 }},
  {{ name = "ground", char = "#", tiles = [0x01, 0x01, 0x01, 0x01], solid = 1 }},
  {{ name = "goal", char = "G", tiles = [0x04, 0x05, 0x14, 0x15], solid = 0 }},
  {{ name = "earth", char = "=", tiles = [0x06, 0x06, 0x06, 0x06], solid = 1 }},
]

# The stage: {WIDTH} cells wide, {HEIGHT} high.
[[asset]]
name = "level"
format = "level-grid"
metatiles = "metatiles"
rows = [
{rows}]
"##,
        name_q = q(name),
        title_q = q(title),
        rows = rows.concat(),
    )
}

/// The pictures, 128 by 16 pixels (16 tiles a row, two rows), one
/// character a pixel: `.` is color 0, `1` to `3` the palette's others.
/// Tile 1 is a block of ground; tiles 2, 3, 18 and 19 the player, a square
/// with two eyes; tiles 4, 5, 20 and 21 the goal's flag on a pole; tile 6
/// earth, the ground with no grass on top.
fn tiles_pix() -> String {
    let (w, h) = (128, 16);
    let mut px = vec![vec![b'.'; w]; h];
    // Ground: a light top edge over a filled block with a few dots.
    for (y, row) in px.iter_mut().enumerate().take(8) {
        for x in 0..8 {
            row[8 + x] = if y == 0 {
                b'2'
            } else if (x + y) % 4 == 0 {
                b'3'
            } else {
                b'1'
            };
        }
    }
    // Earth: the ground's block with no grass on top (tile 6).
    for (y, row) in px.iter_mut().enumerate().take(8) {
        for x in 0..8 {
            row[48 + x] = if (x + y) % 4 == 0 { b'3' } else { b'1' };
        }
    }
    // The player: a square with an outline and two eyes.
    for (y, row) in px.iter_mut().enumerate().skip(1) {
        for x in 1..15 {
            let edge = x == 1 || x == 14 || y == 1 || y == 15;
            let eye = (y == 5 || y == 6) && matches!(x, 5 | 6 | 9 | 10);
            row[16 + x] = if eye {
                b'2'
            } else if edge {
                b'3'
            } else {
                b'1'
            };
        }
    }
    // The goal: a pole and a checked flag.
    for (y, row) in px.iter_mut().enumerate() {
        for x in 0..16 {
            let c = if x == 2 || x == 3 {
                b'3'
            } else if (4..=13).contains(&x) && (1..=7).contains(&y) {
                if (x + y) % 2 == 1 { b'1' } else { b'2' }
            } else {
                continue;
            };
            row[32 + x] = c;
        }
    }
    let mut out = String::from(
        "# Drawn by gp game new --empty: a block of ground (tile 1), the player\n# (tiles 2, 3, 18, 19), the goal's flag (tiles 4, 5, 20, 21) and earth (tile 6).\n# One character a pixel: . is color 0, 1 to 3 the palette's others.\n",
    );
    for row in px {
        out.push_str(std::str::from_utf8(&row).unwrap_or_default());
        out.push('\n');
    }
    out
}

fn template_toml(name: &str, title: &str) -> String {
    format!(
        r#"# Template manifest for {title}, a game started from nothing (gp game
# new --empty). An original (docs/originals.md): its assets are written in
# authored.toml, and `gp template build --write` keeps the pins below.
[template]
format = "0.1.1"
id = {name_q}
title = {title_q}
tier = "open"
model = "model.toml"
contract = "fidelity.toml"

[source]
id = {name_q}
platform = "nes"
how = "authored"
accepts = [""]

[recipe]
asset_map = "asset-map.toml"
asset_map_sha256 = ""
lifted_sha256 = ""

[slots]
level = "layout"
metatiles = "tileset"
palettes = "palettes"
tiles = "graphics"
"#,
        name_q = q(name),
        title_q = q(title),
    )
}

fn fidelity_toml(name: &str, title: &str) -> String {
    format!(
        r#"[contract]
schema = "0.2.0"
model = {}
default = "metric"
description = "{title} as written: held to its own model, the look free"

[[rule]]
path = "levels"
level = "structural"

[[rule]]
path = "rules"
level = "exact"

[[rule]]
path = "assets"
level = "style"

[[rule]]
path = "entities[*].sprite"
level = "style"
"#,
        q(name)
    )
}

fn profiles_toml(title: &str) -> String {
    format!(
        "# Profiles for {title} (docs/formats/profiles.md). The game has no\n# switches yet, so \"normal\" is the game as written.\nformat = \"0.2.0\"\n\n[presets.normal]\ndescription = \"The game as written: it has no switches yet\"\nswitches = []\n"
    )
}

fn rules_toml(title: &str) -> String {
    let gx = goal_x();
    let (sx, sy) = START;
    format!(
        r#"# The level kit's rules for {title} (docs/level-kit.md), written by gp
# game new --empty from the model it wrote. `gp level kit` puts them into
# kit.toml and level-kit.md. Change them when the model changes.
format = "0.1.0"
summary = """
A stage is one level one screen wide ({WIDTH} by {HEIGHT} cells of {CELL} px). The player walks and jumps from the left and reaches the goal at the right. Falling below the level starts the stage again.
"""
open = [
  "The play measures a stage should land near aren't taken: the game is new and has none yet",
  "Originality has no base to compare with: the game was started from nothing",
]

[[data]]
what = "The level"
rule = "A grid of {WIDTH} by {HEIGHT} cells, one metatile a cell: 0 air, 1 ground with grass on top, 2 the goal's flag, 3 earth (the characters . # G = in authored.toml)"
slot = "level"
source = "authored.toml, the level asset"

[[data]]
what = "The metatiles"
rule = "4 metatiles of 4 tiles each, with a solid byte: solid cells (ground and earth) stop the player"
slot = "metatiles"
source = "authored.toml, the metatiles asset"

[[code]]
what = "The goal"
rule = "The player's left edge at x {gx} or past"
source = "the model's levels.main.goal"

[[code]]
what = "Falling out"
rule = "A body below y 208 is out and the stage starts again"
source = "the model's kill_below and rules"

[[code]]
what = "The player's start"
rule = "x {sx}, y {sy}: the start cell must have solid ground under it"
source = "the model's spawn"

[[limit]]
what = "Cells"
limit = "{WIDTH} by {HEIGHT}, {cells} cells"
source = "authored.toml (level: {WIDTH} by {HEIGHT})"

[[limit]]
what = "A jump's reach"
limit = "about 48 px up, 40 px across walking and 80 running (gp interp reach); a gap wider or a step higher can't be crossed"
why = "every stage must stay reachable: gp interp reach checks it"
source = "the model's run and jump"

[[verify]]
step = "1. Reachable"
how = "gp level verify on the game: the goal reachable by the player alone, and a run that reaches it"
"#,
        cells = WIDTH * HEIGHT,
    )
}

fn agents_md(name: &str, title: &str) -> String {
    format!(
        r#"# {title}

A new game started from nothing with `gp game new {name} --empty`: one stage of floor, a player that walks, runs and jumps, and a goal at the right. Its pictures are its own, drawn as plain shapes. It has no sound and no enemies yet.

## Files

| File | Holds |
|---|---|
| `model.toml` | The model (docs/model.md): the player, the stage, the camera and the rules |
| `authored.toml` | The assets as data: tiles, palettes, metatiles and the stage (docs/originals.md) |
| `tiles.pix` | The pictures, one character a pixel |
| `template.toml` | The manifest; `gp template build --write` keeps its pins |
| `asset-map.toml` | Written by `gp template build`. Don't edit it |
| `fidelity.toml`, `profiles.toml` | The contract and the profiles |
| `level-kit/rules.toml` | The level kit's rules; `gp level kit` writes `kit.toml` and `level-kit.md` from them |
| `themes/kit/kit.toml` | The theme kit: each picture a theme draws. After adding an element, run `gp theme kit` on the game again |

## Grow it

| To | Run |
|---|---|
| Check it | `gp validate {name}` |
| Change the stage | `gp level show {name}`, then `gp level edit {name} --op "set row level ROW V,V,.."` (0 air, 1 ground, 2 the goal's flag, 3 earth); `gp level undo {name}` |
| Check the stage | `gp level verify {name}` |
| Add an element (an enemy, a coin) | MCP `model_edit` on the copy `{name}`: an entity under `entities.NAME` and a spawn in `levels.main.spawn` |
| Give it a look | `gp theme look new DIR --game $GP_HOME/work/mcp/{name} --name local/NAME --title "..."` |
| Change the pictures by hand | Edit `tiles.pix` and run `gp template build $GP_HOME/work/mcp/{name} --write` |
| Build it to play | MCP `build_publish` on `{name}` |
| Share it | `gp game share $GP_HOME/work/mcp/{name}` |
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stage_is_floor_with_a_goal_on_it() {
        let rows = level_rows();
        assert_eq!(rows.len(), HEIGHT);
        assert!(rows.iter().all(|r| r.len() == WIDTH));
        assert_eq!(rows[HEIGHT - FLOOR_ROWS], "#".repeat(WIDTH));
        assert_eq!(rows[HEIGHT - 1], "=".repeat(WIDTH));
        assert_eq!(rows[HEIGHT - FLOOR_ROWS - 1].find('G'), Some(GOAL_COLUMN));
        // The start stands on the floor.
        assert_eq!((START.1 + CELL) / CELL, HEIGHT - FLOOR_ROWS);
    }

    #[test]
    fn the_pictures_are_whole_rows_of_pixels() {
        let t = tiles_pix();
        let rows: Vec<&str> = t.lines().filter(|l| !l.starts_with('#')).collect();
        assert_eq!(rows.len(), 16);
        assert!(rows.iter().all(|r| r.len() == 128));
        assert!(
            rows.iter()
                .all(|r| r.bytes().all(|b| matches!(b, b'.' | b'1' | b'2' | b'3')))
        );
    }

    #[test]
    fn a_title_comes_from_the_name() {
        assert_eq!(title_of("my_game"), "My game");
    }
}
