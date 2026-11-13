# Image format config

People edit sprites in tools of their own, such as Pixelorama or Aseprite, or by having an agent write pixels. The image format config says how frames go into image files for them and come back: the layout (one sheet or a file per frame, and the grid) and the scale. It also sets the pixel type (indexed or RGBA) and the palette file. Transparency, names and state tags are set there too, along with the editor. `crates/gp-imagefmt` is the writer and the reader, and `crates/gp-imagefmt/src/config.rs` the definition. This is format 0.1.0, and `schema/image-config.json` its JSON Schema (`gp self schemas` writes it).

Brought from game-printer's docs/formats/image-config.md; the format is unchanged. `gp sprites schema` prints its schema. `gp assets open` writes an entity's frames in the layout it says and brings each save back through the checks below, after starting the editor ([sprite editing](../sprite-editing.md)). `gp mcp`'s `assets_export` with an `editor` writes an entity's frames for that editor too, and `assets_edit`'s `import` op reads the folder the editor saved into back through the same checks ([mcp.md](../mcp.md)). All of them are NES only: a sheet holds four palettes of four colours with colour 0 clear, and an arcade game's (Tapper's Midway MCR) sprite is one 32 by 32 picture of sixteen values, so they refuse an arcade game in words. `gp sprites export` and `gp sprites import`, which wrote a template's frames out and read them back without an editor or a copy, were removed on 2026-11-13.

The exported colours are the NES palette from gp-present; game-printer took them from gp-gen-bevy, which is retired.

## Layers

A config is resolved from layers, each setting only the keys it names, the last winning:

1. The defaults below.
2. `$GP_HOME/config/image.toml`: a person's own.
3. The project's `gp.toml`, its `[image]` table: the first `gp.toml` found from the current directory up.
4. Within each layer, `[templates.<template>]` for the template exported, then its `[slots.<slot>]` (an entity's frames are the slot `<entity>`).
5. A file given with `--config`.

`assets_export` reads layers 1, 2 and 4: the defaults, then `$GP_HOME/config/image.toml` with its `[templates.<source>]` for the copy's model `source` (such as `donkey-kong-oe`) and its `[slots.<entity>]` for the entity exported, then the `editor` asked for (`pixelorama` by default, which also picks Pixelorama's palette file when the layers left `gpl`). `gp assets open` reads all five (`--config` for the last); the crate's `resolve` takes any layers it is given.

```toml
# $GP_HOME/config/image.toml
layout = "sheet"
color = "indexed"
palette_file = "gpl"

[templates.donkey-kong-oe]
scale = 2

[templates.donkey-kong-oe.slots.player]
layout = "frames"
editor = "pixelorama"
```

In a `gp.toml` the same keys sit under `[image]`, `[image.templates.<template>]` and so on.

## Keys

| Key | Values | Default |
|---|---|---|
| `format` | The format version a layer is written for; a layer naming another minor version is refused | `"0.1.0"` |
| `layout` | `sheet` (every frame in one image, on a grid) or `frames` (one image per frame) | `sheet` |
| `order` | `state-frame` (a row per entity and state, its frames left to right) or `frame` (frames in order, `columns` to a row) | `state-frame` |
| `columns` | Columns of a sheet in `frame` order | 8 |
| `cell` | Each cell's `[width, height]`; 0 for the largest frame's | `[0, 0]` |
| `padding` | Pixels between a frame and its cell's edge | 0 |
| `spacing` | Pixels between cells | 1 |
| `scale` | Each pixel drawn this many times across and down, 1 to 16 | 1 |
| `color` | `indexed`: a palette PNG whose entry 4p + c is color c of sprite palette p, entries 0, 4, 8 and 12 transparent; `rgba`: 8-bit RGBA | `indexed` |
| `palette_file` | `gpl` (GIMP palette, read by GIMP, Aseprite, Krita and Pixelorama), `jasc` (JASC-PAL `.pal`), `hex` (one `rrggbb` a line, as Lospec gives), `pixelorama` (Pixelorama's JSON palette), `none` | `gpl` |
| `transparency` | `alpha` (alpha 0, or the transparent entries) or `key` (the `key` color, RGBA only) | `alpha` |
| `key` | The key color, `#rrggbb` | `#ff00ff` |
| `naming` | A frame's name and, in `frames` layout, its file's: `{entity}`, `{state}` and `{frame}` (its number in the sprite set) are filled in | `{entity}_{state}_{frame}` |
| `state_tags` | Tag each entity and state's run of frames in the sidecar | true |
| `editor` | `png` (images and the sidecar), `pixelorama` (the same; pair it with `palette_file = "pixelorama"`), `aseprite` (the sidecar also as Aseprite's sheet JSON, `sprites.aseprite.json`, with `frames` and `meta.frameTags`) | `png` |
| `tile_budget` | Distinct tiles an import may use after flips; 0 for no limit | 256 |

## Files

An export writes `sheet.png` (or one PNG per frame), `palette.<ext>` and `sprites.json`, the sidecar. The sidecar (`gp-sprite-sheet 0.1.0`) holds the resolved config, the 16 sprite palette colors, and each frame's name, entity, state, number in the set, file and rectangle, origin from the entity's position, and tiles (each one's place in the frame, palette and flips), and the state tags. The tiles are what an import checks against, so the sidecar must come back with the images.

## Import checks

An import reads the images through the sidecar and refuses (each problem named with its frame) when:

- the palette has no entry (indexed) or color (RGBA, matched within its tile's palette) for a pixel;
- a tile mixes palettes: every drawn pixel of a tile is a color of the tile's one palette, as a console's sprite attribute byte allows;
- a drawn pixel lies outside the frame's tiles;
- the distinct tiles, where a tile and its mirror count once, are more than `tile_budget`.

The frames that changed come back as an edit of the copy, through every edit check. That holds for `assets_edit`'s `import` op and for each save `gp assets open` sees; the edit is placed, or with `place: false` proposed.

## Editors

| Editor | How |
|---|---|
| Pixelorama | `assets_export` with `editor: "pixelorama"` writes the set and a `gp-edit.json` trip file beside it, and `assets_edit` with `import` brings the saved folder back as a checked edit. `gp assets open <copy> <entity> --editor pixelorama` also starts Pixelorama and watches for saves. By hand: import `sheet.png` as a spritesheet with the sidecar's cell size (or open the per-frame PNGs), and load `palette.json` (`palette_file = "pixelorama"`) or `palette.gpl`. Save PNGs over the exported ones. Its `.pxo` project format isn't written yet: it is a ZIP of JSON and image data whose layout we haven't pinned (an open question) |
| Aseprite (and LibreSprite, `editor: "aseprite"` or `"libresprite"`) | Open `sheet.png` and use Import Sprite Sheet with the cell size, or open the frames; `sprites.aseprite.json` has the frame rectangles and tags as Aseprite's own sheet export writes them. The `.aseprite` binary format isn't written |
| Any editor | Edit the PNGs in their own palette (indexed) or colors (RGBA) |

`assets_place` (docs/mcp.md) also takes an RGBA image of a tiles slot through this crate, reading the four preview grays as indices.
