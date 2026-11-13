# Level chunks

> Brought from game-printer with wave 9 of the port. gameprinter cut other games' code, so the readers of Super Mario Bros.', Super Mario Bros. 2's and Mega Man 2's layouts are gone; their rows below, and the results and notes on those games, are game-printer's record and are kept as they were. New here: a reader for a level drawn by a PPU string, so Donkey Kong's four stages are read (game-printer refused them: "a layout from a ppu-strings asset isn't read yet"), and `--module`, the game's built module that the reachability tests step the model through, as `gp interp reach` does. A template is a folder, an id under `templates/` or a game in the games checkout (`gp chunks donkey-kong --module ...`). An arcade game is read the same way once its model's levels name a layout; Tapper's model has no level layout yet, so `gp chunks tapper` says that its level names no layout or grid slot. The `chunks.<template>.*` metrics aren't recorded, since `gp chunks` doesn't hand them to the run recorder yet.

A level chunk is a group of cells that a game's levels use again and again in the same arrangement: a pipe of some height, a row of ? blocks, a gap two cells wide, a staircase, a ledge with a walker on it. `gp chunks` finds a game's chunks, says what each one does for the player, and writes them as a catalog ([the format](formats/chunks.md)). It detects and catalogs only. A level generator that builds new levels from the catalog comes later (the user's rule: detect before fixing), and the end of this page says how it would use it. It's Milestone 2 work, beside [gaps](gaps.md) and the [mechanics](mechanics.md) catalog; the research it serves is [the mechanics research](research/mechanics.md), whose clean-room model needs new levels that play under the same mechanics.

```sh
gp chunks donkey-kong --module ~/.gameprinter/work/modules/donkey-kong.wasm   # every stage; the catalog goes to $GP_HOME/work/chunks/<template>/
gp chunks templates/mosshop --min-count 1 --json
gp chunks --schema                             # the catalog format's JSON Schema
```

| Option | Meaning |
|---|---|
| `--assets` | The model's lifted assets (default `$GP_HOME/work/assets/<source>`; `gp assets lift` or `gp template build` makes them) |
| `--level` | Only these model levels (default all) |
| `--interactive` | The interactive element catalog to match cells against (default `$GP_HOME/reports/gaps/<template>/interactive.json` when it exists, from `gp gaps`) |
| `--min-count` | Occurrences a group, pattern or compound needs to be a kind (default 2; 1 for a game with one level) |
| `--max-patterns` | Window patterns kept per size (default 200) |
| `--reach-tests` | Kinds per layer given a reachability test (default 6, 0 for none) |
| `--top` | Kinds printed per layer (default 8); the catalog keeps all |
| `--out` | Where to write the catalog; refused inside a git work tree |

The metrics a run is to record (not yet, above) are `chunks.<template>.kinds`, `.objects`, `.groups`, `.gaps`, `.patterns`, `.compounds` and the mean coverage `chunks.<template>.coverage` (and `.coverage.objects`, `.groups`, `.patterns`), which the [scorecard](scorecard.md) shows in its `chunks` column.

## What it reads

A level is read as a grid of cells. Everything below works on that grid, so it's the same for every game; only reading the grid depends on the level's format, and that is chosen by the layout slot's asset kind, not by the game.

| Fact | From |
|---|---|
| Each cell's collision class | The compiled model's level (`gp_interp::compile`): `empty`, `solid`, `top`, `ladder`, `hazard`, `water`, `ice`, as the simulation sees it |
| Entities placed, and whether falling kills | The compiled level's spawns (the player left out) and its `kill_below` |
| Each cell's value, its name and tile kinds | The layout's format (the table below) |
| Each value's four tile numbers | The format's metatile tables, for matching [interactive element](formats/interactive.md) kinds |
| The objects and the cells each drew | The format's object list, when it has one |

| Layout asset kind | Cell value | Tile kinds | Objects |
|---|---|---|---|
| `level-grid` with a `metatiles` tileset (Moss Hop, the originals) | The grid's metatile number, named by the tileset | none beyond the class | none |
| `smb-area-objects` | The block buffer's metatile (`gp_assets::smb_area`), named by the metatile sets | coin, hidden, bumpable, breakable, climbable, the contents of a block, pipe parts (`AreaTables::properties`) | The area's objects and its header |
| `smb2-level-objects` | The area's tile (`gp_assets::smb2_area`), its tiles from `tile_quads_1` to `_4` | top only, climbable (the collision tables), doors, jars and light entrances by cell | The level's objects and its header |
| `mm2-screens` | The 16-pixel quadrant's byte of its metatile (two collision bits and a tile group) | none beyond the class | none: the screens are raw metatiles |
| `ppu-strings` (Donkey Kong, gameprinter only) | The tile the string writes on the first name table, 0 where nothing is drawn; the level is the name table's 32 by 30 cells | none beyond the class | Each update of the string: a `row` or `column` of tiles, with `x`, `y` and `length` |

Levels that share a layout (Super Mario Bros.' 56 levels use 34 areas) are read once, with every level name kept.

## Two capture layers

### The format's own objects

When a level is an object list, the list's object kinds are the game's own vocabulary of chunks, chosen by its designers: Super Mario Bros.' area objects (a pipe of height h, a row of bricks of length n, a staircase, a hole) and Super Mario Bros. 2's level objects (horizontal and vertical blocks, platforms, vines, doors, jars). Each object type becomes a parametric chunk kind.

An object's cells are found the same way for every format: the list's builder runs on the list's first k units and on its first k+1, and the object's cells are those that changed, with the values they got. The header is the first unit, and its cells are those of the header alone that aren't empty space (Super Mario Bros.' terrain and scenery from the header bits). So an object's footprint is the game's own expansion of it, through the builders the interpreter and the round-trip checks use (`smb_area::build_area`, whose solidity is checked cell for cell against the game's block buffer on 31 areas, and `smb2_area::build_area`, checked against 83 areas' tiles in PRG RAM), with no expansion rule written twice.

A kind's name comes from the format's module (`gp_assets::smb::area_object_kind`, `gp_assets::smb2_area::object_kind`), without the size, and its parameters are the size field read as the format reads it (`length`, `height`, `size`, a ground `setting`). For each parameter the catalog keeps the commonest footprint for every value seen and, when they lie on one line, the line: on Super Mario Bros. a pipe is 2 cells wide and `height` high, a row of bricks, coins, solid blocks or ? blocks `length` wide, a column `length` high, a hole `length` wide and 2 deep, a staircase `length` wide (and as high), all fitted exactly from the levels. Objects that changed no cell are counted as controls instead: page jumps and area pointers, scenery the block buffer doesn't keep (castles, ropes), and objects drawn over entirely by later ones.

### Patterns mined from the grid

Any format's grid, and the only route for formats with no object list (Mega Man 2's screens, the originals' grids), is mined in four ways, in the line of the machine-learned level generators that Summerville, Snodgrass, Guzdial, Holmgard, Hoover, Isaksen, Nealen and Togelius survey ("Procedural Content Generation via Machine Learning (PCGML)", IEEE Transactions on Games 10(3), 2018), which learn from tile grids of existing levels.

| Kind | What | After |
|---|---|---|
| Groups | Connected pieces (4-neighbour) of the level's structure that aren't terrain, keyed by their exact shape and cell values, up to 16 by 16 cells; a shape seen at least `min_count` times is a kind | The objects as the grid shows them: Super Mario Bros.' pipes come out as pipe groups without the object list |
| Gaps | Runs of columns with nothing at the bottom, in a level where falling kills and the bottom row is at least a quarter floor, keyed by width | The pits Smith et al.'s rhythm groups place (Launchpad) |
| Windows | Every 2x2, 3x3 and 4x4 window holding some structure, counted across all the game's levels; the most frequent 200 of each size with two or more occurrences, mixed windows before plain floor | The overlapping model of Maxim Gumin's WaveFunctionCollapse (2016, github.com/mxgmn/WaveFunctionCollapse), which learns N by N patterns and their overlaps from a sample; Snodgrass and Ontanon's multi-dimensional Markov chains over tile neighbourhoods ("Experiments in map generation using Markov chains", FDG 2014) |
| Slices | Whole columns of a level that runs across (rows of one that runs down), counted the same way | The vertical slices of Dahlskog, Togelius and Nelson's n-gram levels ("Linear levels through n-grams", MindTrek 2014) and the column strings of Summerville and Mateas ("Super Mario as a String: Platformer Level Generation Via LSTMs", DiGRA and FDG 2016); the tile-grid level representation is the Video Game Level Corpus's (Summerville, Snodgrass, Mateas and Ontanon, "The VGLC: The Video Game Level Corpus", PCG workshop 2016) |

Structure is any cell that isn't empty to collision or has a tile kind (a coin is empty to collision and is structure). Terrain is the floor: the values seen in the stack of structure rising from the bottom of at least half the columns (from the sides, for a level that runs down) and the commonest structure value, where they join an edge of the level through cells of those values. Pieces bigger than 16 by 16 are left to terrain and counted in the notes.

### Groupings above the objects

Compounds are two pieces, or a piece and an entity, that sit together the same way at least `min_count` times. The pieces of a level are its objects (its groups, for a format without objects; objects wider than two screens set terrain and aren't pieces) and its gaps.

| Relation | When |
|---|---|
| `with` | An entity is placed on or near the piece: within a cell on either side, up to three above or one below ("pipe with piranha_plant") |
| `over` | The piece is above a gap, sharing a column ("row of bricks over gap 3 wide") |
| `above` | The piece is above another, sharing a column, up to four cells apart ("row of coins above row of bricks") |
| `left-of` | The other piece starts up to two cells to its right, sharing a row ("column of solid blocks left of column of solid blocks", a staircase built of columns) |

## Function

Every kind gets its function, worked out over its example and its occurrences:

- the example's cells by collision class and by the format's tile kinds;
- interactive element kinds, by matching each cell's four tile numbers with the `before` of each kind in the gaps catalog, so a brick row's cells match `cell 45454747 -> 24242424`, the brick that breaks; a kind that starts from empty space's tiles isn't matched;
- the entities placed on or near its occurrences and how often;
- its placement: whether it rests on something, where it sits in the level (by thirds of height and length) and where on a screen (by thirds of 16 cells);
- what sits next to it on each side, by chunk id, or terrain, the level's edge or nothing;
- for the commonest kinds of each layer but patterns (`--reach-tests`, default 6), what it demands.

The demand comes from `gp_interp::reach`, the search over the player's own moves in the simulation ([mechanics](mechanics.md)). For each tested kind a made-up level is built in the compiled model: the chunk's level's height, a floor two cells deep, eight open cells on each side, the example's cells pasted where they were (moved down onto the floor when it rests on something), the player on the left and the goal on the right, with the model's hooks. The search answers whether the goal is reachable past the chunk and whether the player stood on it. With the chunk's shape (the highest wall rising from the floor, whether it blocks the player's body at the floor, the widest gap at the bottom) and the jump's envelope that gives `needs`: `walk`, `jump`, `running jump` (a gap wider than a walking jump carries) or `blocked`. Because it runs the model's movement, a model with a higher jump gets different answers with no code changed.

## Results

These runs were made on 2026-10-29 into a scratch `$GP_HOME` with the defaults. They read the lifted assets in `$GP_HOME/work/assets/` (for Super Mario Bros. 2, lifted from the ROM for the run) and the gaps catalogs `gp gaps` wrote from the store's recordings c8f4e908, 35a900f9 and c31a0471. The coverage figures are means over a game's layouts, and the groups figure counts only cells that aren't terrain.

| Game | Layouts | Objects | Groups | Gaps | Patterns | Compounds | Coverage: objects, groups, any | Reach tests |
|---|---|---|---|---|---|---|---|---|
| Super Mario Bros. (every area) | 34 | 38 | 94 | 13 | 800 | 182 | 100%, 56%, 100% | 24: 14 jump, 8 walk, 2 running jump |
| Super Mario Bros. 2 (1-1's areas) | 6 | 32 | 7 | 3 | 579 | 44 | 100%, 10%, 100% | none (below) |
| Mega Man 2 (Wood Man's rooms) | 14 | none | 10 | 2 | 585 | 15 | none, 46%, 100% | 13: 10 jump, 2 walk, 1 blocked |
| Moss Hop | 1 | none | 0 (3 at `--min-count 1`) | 1 | 31 | 0 | none, 0%, 100% | 1: jump |
| Badge In | 3 | none | 6 | 4 | 233 | 1 | none, 62%, 100% | 11: 4 jump, 2 running jump, 2 walk, 3 blocked |
| Signal Failure | 3 | none | 3 | 2 | 215 | 1 | none, 49%, 100% | 6: 3 walk, 2 jump, 1 running jump |
| Swing State | 3 | none | 6 | 4 | 248 | 4 | none, 82%, 100% | 14: 11 jump, 2 walk, 1 running jump |

Pattern coverage is close to 100% everywhere by construction. Small windows repeat across a game's levels, so the object and group columns are the useful measures. The Super Mario Bros. catalog keeps the full 200 windows of each size and 200 slices.

### Super Mario Bros.

The object layer explains every structure cell of every area. The commonest kinds and what they do:

| Kind | Occurrences | Function |
|---|---|---|
| column of solid blocks (`length` = height) | 174 in 24 areas | solid; on the ground 153 times; a column 4 high needs a jump (a 64 pixel wall, under Mario's 71 pixel rise) |
| row of coins (`length` = width) | 143 in 20 | coins, matching the gaps catalog's coin kind (`a5a6a7a8 -> 24242424`); in the air 86 times |
| row of bricks | 139 in 17 | breakable and bumpable, matching the breaking brick; in the air 133 times; walked under |
| hole (`length` = width, 2 deep) | 119 in 21 | a gap in the floor: a jump clears 2 cells (32 pixels) |
| pipe (`height`) | 92 in 18 | pipe shaft and top; a piranha plant at 95% of them; a pipe 2 high needs a jump |
| tree ledge | 61 in 4 | always in the air (the ledge's top), a red Koopa near 10% |
| gap 2 wide, 1 wide, 3 wide | 52, 40, 25 | a jump; 6 wide (96 pixels) needs a running jump, since a walking jump carries 88 pixels |
| pipe with piranha_plant | 90 in 17 | the compound the user named, found from the spawn list |
| column of solid blocks left of column of solid blocks | 91 in 20 | the staircases built of columns; needs a jump |
| row of coins above row of bricks, row of bricks above row of bricks | 19, 27 | the stacked block rows of 1-1 and 1-2 |
| used block with firebar_ccw | 17 in 3 | the castles' firebar pivots |

Groups find the same pipes without the object list (2 by 2, 2 by 3 and 2 by 4 pipe groups, 24 each, a piranha plant at over 92%), single bricks and coins, hidden coin blocks (bumpable, hidden, contents coin) and the flagpole (climbable, 16 times in 16 areas).

### Super Mario Bros. 2

Every cell of 1-1's six areas is inside one of its level objects. The commonest kinds are green platforms (35, top-only cells, with a pink Shy Guy at 43% of them) and ground settings (the format's terrain changes, 40 cells wide); vines are climbable. Compounds put the enemies on them: "green platform with shy_guy_pink" (22), "with ninji_jumping" (13), "with tweeter" (5). Groups are few (7) because most of the structure is the ground setting's terrain and tall platforms over 16 cells. The reachability search doesn't run: the model's player has no `run` component (its hook moves it), and `gp_interp::reach` searches only a player with one (an open question below).

### Mega Man 2

With no object list, everything comes from the grid. Groups are the walls of the shafts and the blocks of the rooms. Solid columns 3 to 5 cells high are the commonest (47 in all), and Mega Man's 50 pixel rise climbs those of 3 and 4; a full-height wall is `blocked`. Single blocks have Friender or Batton near, and ladders are 4 `ladder` cells high. Compounds are blocks beside blocks (the stepped walls) and the long gap of the Monking room with its enemies. Interactive matches here are mostly the room transitions' redraws, which the gaps catalog lists as cell changes.

### Moss Hop and the originals

Moss Hop's one level gives one gap 2 wide (a jump) and, with `--min-count 1`, its stone blocks: a row of three a slug walks under, and walls one and two high that a jump climbs and the player stands on. The originals' grids give their props as groups: Badge In's desks (a coffee pickup near a third of them) and filing cabinets (walls 1 to 3 high), Signal Failure's ticket barriers and benches (a free paper near), Swing State's lecterns, each with a pundit (10 of 10), and bunting over gaps. Badge In's gaps 10 and 11 wide are `blocked` alone and crossed by the desks over them, which the compound "desk x3 over gap 11 wide" records; a generator has to keep that pair together.

## What is general and what is per game

| Part | General | Per game (as data) |
|---|---|---|
| The grid, classes and spawns | One reader for every model, through the compiled level | none |
| Cell values, names, tile kinds | One reader per layout asset kind (grid, `smb-area-objects`, `smb2-level-objects`, `mm2-screens`); a new game in a known kind needs nothing | The formats' tables, lifted by the asset map |
| Objects and footprints | One method for any object-list format: builds of the list's prefixes, by the format's own builder | The format's unit splitter and kind names, beside the format's parser in gp-assets |
| Groups, gaps, windows, slices, compounds | Rules on the grid that know no game | The thresholds in `settings` |
| Function | Classes, kinds, interactive matches by tile numbers, spawns, placement, adjacency; reachability through the model's own movement | The gaps catalog's kinds, the model's mechanics |

How each piece is found for any game: a new object-list format needs a builder (which the interpreter needs anyway) and a function that splits its list into units and names each unit's kind; a raw-tile format needs only its grid reader. The vocabulary differs by game in what it's made of, not in how it's found. Super Mario Bros.' chunks are its designers' parametric objects, and its levels are rows of them over a floor with holes. Super Mario Bros. 2's are world-specific objects (the same type is a whale or a drawbridge chain by world) over ground settings, in areas that also run vertically. Mega Man 2's are fixed metatile screens, so its only chunks are the mined ones, walls, ladders and blocks, at the 32-pixel metatile grid. The originals are grids of a few props. Gaps, groups and the `with` compound are found in all of them.

## How a level generator would use the catalog

A generator for a clean-room model or a "make me a harder level" request (Milestone 3) would take the catalog as its pieces and rules:

1. Pieces: object kinds with their parameters and footprint lines, where the format has them (the generator writes objects, and the format's builder expands them, so the level is in the game's own format); groups and window patterns otherwise.
2. Placement: each kind's placement counts as a prior on where it goes, and the adjacency counts and compounds as what may stand next to what. A pipe gets a piranha plant 95% of the time; coins go above bricks; desks go over the wide gaps.
3. Rhythm and difficulty: gaps by width and walls by height, with each kind's demand, give the pieces' difficulty; segment counts say what comes early and what late.
4. Combination rules that keep it playable: a gap or wall is placed only within the envelope (`gap_px` under the run distance, `height_px` under the rise), a `blocked` kind only with the compound that crosses it, and every generated level is checked whole with `gp_interp::reach` before it's kept, as the mechanics research's clean-room check says.
5. Difference: a clean-room generator works from the catalog's kinds and statistics and never from the original's levels. The window overlap measure of the mechanics research checks that the result differs.

The catalog is the original's content and stays on the person's machine; only the generator's rules and the levels it makes may leave.

## Limits and open questions

- The reachability search needs a `run` component, so Super Mario Bros. 2's hooked player isn't searched; it ignores enemies, and a landing restarts with no speed (docs/research/mechanics.md).
- A made-up level is a flat floor; a chunk that needs what's around it (a platform reached from a ledge) is tested alone.
- Groups and patterns are keyed by exact cell values, so the same shape in another area's metatiles is another kind.
- Patterns are capped per size, and their coverage is close to 1 by construction.
- Moving platforms, vines that grow, and Super Mario Bros. 2's jars and subspace aren't in the grid, so they aren't chunks.

| Question | Bites at | Recommended default |
|---|---|---|
| Should groups and patterns also be keyed by class? | M2.8 | Filed 2026-10-29 by m2/chunks. Keyed by exact values, Super Mario Bros.' brick row in a ground and an underground area are two shapes. Default: add a second key, the shape in collision classes and tile kinds (`solid`, `coin`, `bumpable`), and report both, so a generator can pick the class shape and draw it in the area's own metatiles |
| How does the reachability search take a player without `run`? | M2.8 | Filed 2026-10-29 by m2/chunks. `gp_interp::reach` builds its pad from `run`; Super Mario Bros. 2's player moves by a hook. Default: build the pad from the model's input actions (left, right, jump) when there's no `run`, and let the hook move the player, as `reach` already runs hooks |
| Should a chunk's test include its surroundings? | M2.8 | Filed 2026-10-29 by m2/chunks. A chunk is tested alone on a flat floor. Default: also test each compound as a pair, and a window of the source level around the example (one screen), and record both answers |
| What turns the catalog into a level generator? | M3 | Filed 2026-10-29 by m2/chunks. Default: a generator over object kinds (formats with objects) or groups (raw grids) that samples kinds by segment, places them by the adjacency and compound counts, keeps gaps and walls inside the envelope, writes the level in the format, and keeps it only when `gp_interp::reach` finds the goal; Moss Hop first, in `just check` |
| How much of a catalog may a clean-room generator see? | M3 | Filed 2026-10-29 by m2/chunks. The catalog holds the original's shapes. Default: the generator sees kinds' functions, parameter ranges, footprint lines, placement and adjacency statistics and demands, and never shapes or examples; a clean-room level is drawn in its own tileset |
| Should chunks be mined from recordings too? | Later | Filed 2026-10-29 by m2/chunks. Moving platforms, grown vines and runtime tile changes are only in play. Default: when a format can't say them, mine the nametable cells `gp gaps` reads, the same way, and join kinds by position |
