# Level chunk catalog, version 0.1.0

`gp chunks` writes what it finds in a game's levels as a catalog of chunk kinds: the groups of cells the levels are built from, at the format's own object level and as patterns mined from the decoded grids, each with what it does for the player and how it sits beside others. [Chunks](../chunks.md) says how they're found and what a level generator would use them for. The catalog changes no model.

The catalog is `chunks.json` under `$GP_HOME/work/chunks/<template>/` (or `--out`). It holds a game's level content (cell values, shapes, positions), so it's game-derived like the lifted assets and is never written inside a git work tree: `gp chunks` refuses an `--out` there. The repo holds only this format and the code. The JSON Schema is `schema/chunks.json`, generated from the Rust types in `crates/gp-chunks/src/format.rs` by `gp chunks --schema` (`just docs` writes it), and a test fails when it's stale. The format is outside the fixed core, so a change bumps its own version only.

## File

The file is one JSON object.

| Key | Meaning |
|---|---|
| `format`, `version` | `gp-chunks` and `0.1.0` |
| `template`, `model`, `source` | The template's folder name, the model's name and its source |
| `settings` | What it was mined with: `min_count` (occurrences a group, pattern or compound needs, default 2), `windows` (window sizes, default 2x2, 3x3 and 4x4), `slices` (whether one-cell slices across the level were mined), `max_patterns` (kept per size, default 200), `max_group` (the largest group, default 16 by 16 cells), `reach_tests` (kinds per layer given a reachability test, default 6) |
| `envelope` | The player's jump on a flat floor, `{rise, airtime, walk_distance, run_distance}` in pixels and frames (`gp_interp::reach::envelope`), or null when the player can't be searched |
| `interactive` | The interactive element catalog read ([interactive elements](interactive.md)), or null |
| `levels` | Each layout read (below) |
| `layers` | Chunk kinds per layer: `objects`, `groups`, `gaps`, `patterns`, `compounds` |
| `controls` | Objects of the format that changed no cell, by kind and count: page jumps, area pointers, scenery the grid doesn't hold, and objects a later one drew over entirely |
| `chunks` | The kinds (below), by layer in the order of `layers`, then most occurrences first; in `patterns`, those of one value (plain floor) come after the mixed ones |
| `notes` | What couldn't be done and why: a level not read, pieces left to terrain, a player that can't be searched |

A level entry:

| Key | Meaning |
|---|---|
| `name`, `levels` | The first model level with this layout, and all of them (levels sharing a layout are read once) |
| `layout` | `<slot>/<entry>` for a layout from an object list or screens, the grid's slot for a grid |
| `size`, `tile_size` | Width and height in cells, and pixels per cell |
| `cells`, `terrain` | Cells of the level's structure (not empty to collision, or with a tile kind), and how many of them are terrain |
| `objects`, `spawns` | Objects in its list (0 for a format without one), and entities placed, the player left out |
| `coverage` | `{objects, groups, patterns, any}`: the share of the structure cells inside an occurrence of a cataloged kind of that layer, and of any layer. `groups` is of the cells that aren't terrain |

## A chunk

| Key | Meaning |
|---|---|
| `id` | Unique: `object:<name>`, `group:<hash>`, `gap:<width>`, `window:<w>x<h>:<hash>`, `slice:<hash>` or `compound:<relation>:<a>\|<b>`. A hash is ten hex digits of FNV-1a over the shape |
| `layer` | `objects`, `groups`, `gaps`, `patterns` or `compounds` |
| `name` | The format's name for an object kind; the cell values' names, commonest first, for a group or pattern; `gap N wide`; the parts and relation for a compound |
| `size` | The example occurrence's width and height in cells |
| `shape` | The example's cells by rows from the top: the format's cell value (a metatile or tile number), or null where the chunk has no cell |
| `params` | For an object kind, each parameter (below) |
| `count`, `by_level` | Occurrences in all levels, and by level |
| `relation`, `parts` | For a compound: `with` (an entity on or near the part), `over` (above a gap), `above`, `left-of`, and the two parts' ids; an entity part is `enemy:<entity>` |
| `function` | What it is to the player (below) |
| `examples` | Up to five occurrences, `{level, x, y}`, the top left cell |

A parameter: `name`, `values` (seen, smallest first), `footprint` (for each value, the commonest `size` and the occurrences) and `fit`, a line `{axis, base, per}` with `axis = base + per * value` in cells when every footprint is on it.

`function`:

| Key | Meaning |
|---|---|
| `classes` | The example's cells by the model's collision class: `empty`, `solid`, `top`, `ladder`, `hazard`, `water`, `ice` |
| `tile_kinds` | The example's cells by the format's tile kinds (a coin, a block hit from below, a breakable brick, a pipe top, a door, a climbable tile) |
| `interactive` | The example's cells by interactive element kind, matched by the cell's four tile numbers against the kind's `cell.before`; a kind whose `before` is empty space's tiles isn't matched |
| `enemies` | `[{entity, count, share}]`: entities placed within a cell of an occurrence on either side, up to three cells above or one below; how many, and the share of occurrences with one |
| `demand` | From a reachability search on a made-up level that holds the example, when one ran: `reachable`, `needs` (`walk`, `jump`, `running jump` or `blocked`), `height_px` (the highest wall rising from the floor), `gap_px` (the widest run of columns with nothing at the bottom), `stands_on_top` (whether the player stood on it) and `moves` |
| `placement` | Occurrences that rest on something standable (`ground`) or not (`air`); by the third of the level's height their middle is in (`rows`: top, middle, bottom), by the third of its length (`segment`: early, middle, late) and by the third of a 16-cell screen (`screen`: left, middle, right) |
| `adjacency` | `{left, right, above, below}`: per occurrence, the nearest piece on that side (within 8 cells across or 6 up and down, sharing rows or columns) by its chunk id, else `terrain` or `edge` when it touches one, else `none`. Pieces are the level's objects (or its groups, for a format without objects) and gaps. Patterns and compounds have none |

## Example

Made up, cut to one chunk:

```json
{
  "format": "gp-chunks",
  "version": "0.1.0",
  "template": "example",
  "model": "example",
  "source": "example",
  "settings": { "min_count": 2, "windows": [[2, 2], [3, 3], [4, 4]], "slices": true,
                "max_patterns": 200, "max_group": [16, 16], "reach_tests": 6 },
  "envelope": { "rise": 48, "airtime": 40, "walk_distance": 40, "run_distance": 80 },
  "interactive": null,
  "levels": [{ "name": "main", "levels": ["main"], "layout": "level", "size": [32, 15],
               "tile_size": 16, "cells": 96, "terrain": 90, "objects": 0, "spawns": 2,
               "coverage": { "objects": 0.0, "groups": 0.0, "patterns": 1.0, "any": 1.0 } }],
  "layers": { "objects": 0, "groups": 0, "gaps": 1, "patterns": 31, "compounds": 0 },
  "controls": {},
  "chunks": [{
    "id": "gap:2", "layer": "gaps", "name": "gap 2 wide", "size": [2, 3],
    "shape": [[0, 0], [0, 0], [0, 0]], "params": [], "count": 1, "by_level": { "main": 1 },
    "relation": null, "parts": [],
    "function": {
      "classes": { "empty": 6 }, "tile_kinds": {}, "interactive": {}, "enemies": [],
      "demand": { "reachable": true, "needs": "jump", "height_px": 0, "gap_px": 32,
                  "stands_on_top": false, "moves": 412 },
      "placement": { "ground": 1, "air": 0, "rows": [0, 0, 1], "segment": [0, 1, 0], "screen": [0, 0, 1] },
      "adjacency": { "left": { "terrain": 1 }, "right": { "terrain": 1 }, "above": { "none": 1 }, "below": { "edge": 1 } }
    },
    "examples": [{ "level": "main", "x": 12, "y": 12 }]
  }],
  "notes": []
}
```

## Decisions

| Question | Decision |
|---|---|
| Where the catalog goes | `$GP_HOME/work/chunks/<template>/`, since it holds the game's levels; never in the repo |
| What a cell value is | The format's own (a metatile or tile number), so a shape can be drawn and rebuilt with the game's tables. The collision class beside it is the model's |
| How an object kind is keyed | By the format's name for it without its size, so a pipe of any height is one kind with a `height` parameter |
| How a group or pattern is keyed | By its exact cell values. Keying by collision class instead, which would join the same shape drawn in another area's palette, is an open question |
