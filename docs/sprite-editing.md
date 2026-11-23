# Editing sprites with an agent

Brought from game-printer's docs/sprite-editing.md. On an arcade game's lift (Tapper's) the external editor trip is the board's own sprites, and a save comes back as a proposal on the person's page as an NES tile's does ([Arcade boards](#arcade-boards), [On the page](#on-the-page)).

A person tells their agent what to change about a sprite ("give him a red cap", "a bolder outline", "make the slug face the other way"), and Game Printer does it. The agent turns the request into deterministic operations, and Game Printer checks the result against the model and draws it into the recorded frames where the sprite is seen. The person sees the before and after pictures on their page and accepts or rejects it there (or with `gp assets proposal` in their terminal); the agent can't answer for them. There are no painting tools in Game Printer yet. A person who wants to draw by hand uses an external editor, starting with Pixelorama (phase 3, below).

A person who draws by hand opens the sprite set in Pixelorama with `gp assets open`, and each save comes back the same way as an agent's edit.

The same tool edits a level tile's look ("make the stone blocks orange"): see [Level tiles](#level-tiles).

This page is the product doc. The design and its prior art are in the research report (game-printer's docs/research/agent-sprite-editor.md, not carried here); the tools are `assets_edit` and `assets_history` in [agent tools over MCP](mcp.md). The person answers an `assets_edit` proposal on their page ([On the page](#on-the-page)); a picture of a large-art theme is proposed with `gp theme picture` and answered there too ([A picture of your own theme](#a-picture-of-your-own-theme)).

Everything is local and free: deterministic operations in Rust, the interpreter for previews, and no hosted service. If an image model is ever added, it is a free model run locally behind the generator trait. The editor is fully useful without one.

## What can be edited

An edit reads the model, so a sprite is editable only when the model knows it completely: its tiles, its palette, its layout, the states that draw it, where it's drawn (a level spawns it or a recorded baseline shows it) and evidence on every sprite, size and hitbox fact. `gp_mcp::ready` checks this per entity. When something is missing, `assets_edit` refuses with `stage: "model"`. The reply has the list and one sentence saying how to fill it (usually the capture loop: `gp capture`, or `model_refine`).

The unit of an edit is a frame of the entity's sprite set (`gp_mcp::sprite_set`). A frame is a layout of pieces: each a tile of the slot placed in the picture with its own flips, as a console's sprite attribute bytes give it.

| Frame id | What it is |
|---|---|
| `base` | The sprite's own frame: its tiles in rows of `width` (with `cells`' flips and places), or the frame of a sprite-set it names (`frame = { slot, row }`, written by `gp sprites capture`, [sprite sets](formats/sprite-set.md)) |
| `clip<c>.<f>` | Frame `f` of clip `c`, when it names tiles or a set row of its own |
| `state.<name>` | The sprite a machine state sets (`set.sprite`, merged into the entity's) |
| `state.<name>.clip<c>.<f>` | That sprite's clip frames |
| `set.<n>` | A frame of the sprite-set asset made for this entity that the model doesn't name yet |

Frames with the same pieces are one picture (the other ids are its `aliases`). `frames` takes ids, a clip's name (`clip0`: every frame of it) or `all`. A frame whose pieces are in another tiles slot, or a row of a plain `rows` slot, is listed under `sprite_set.skipped` with why.

An edit is written back piece by piece, each piece's square into its tile with its flips undone. Frames often share tiles. A clip's frames reuse a head, and a body's right half is its left half mirrored. After the ops every edited frame must read back as it was drawn, so an edit that draws a shared tile two ways (one half of a mirrored head) is refused with `stage: "frames"` and the frames named; an edit that agrees (a recolor) holds across the whole clip. `also_changes` lists the frames not picked that draw a changed tile, so the agent can say which other pictures move with it.

## Operations

All operations are deterministic. The same picture and arguments give the same pixels on every machine, with no floats. Each keeps the picture's size, so the tile budget holds, except `resize`, which takes free tiles.

| Operation | What it does |
|---|---|
| `recolor` | Changes color indices, all pairs at once (`[[1,3],[3,1]]` swaps), optionally inside a `region` |
| `flip`, `mirror` | Mirrors the picture, or makes it symmetric from one half |
| `shift` | Moves every pixel, optionally wrapping |
| `outline` | Draws an index around the art (four sides, or eight with `corners`) |
| `fill`, `pixels` | Fills a rectangle, sets single pixels |
| `copy` | Copies another entity's sprite or tiles of the sheet |
| `load` | Reads a PNG of the sprite's own frame an external editor saved |
| `pictures` (argument) | Whole frames by id, as rows of indices: an editor's save, or a frame an agent draws outright |
| `import` (argument) | A folder `assets_export` or `gp assets open` wrote and an editor saved into, read back with the image format's checks into `pictures` |
| `match_style` | Takes another entity's outline: where the reference outlines its art in one index (at least half of its edge pixels), this sprite's edge pixels take that index |
| `tween` | An in-between of two frames: each moves half way toward the other by the centre of its drawn pixels; pixels they agree on are kept and the rest alternate in a checkerboard |
| `resize` (argument) | `[width, height]` in tiles: the art keeps its bottom left (where it stands on screen, since a sprite is drawn from the body's left edge on its bottom edge), and new tiles come from free ones; the model's sprite gets the new tile list and width in the same placement |
| `palette_ops` | On the entity's palette instead of its pixels: `color` (index 1 to 3 to an NES color), `swap`, `hue` (steps of the NES's 12 hues; grays stay), `brightness` (rows of 4; `$0D` becomes `$0F`), `copy_from` another entity; `same_role` applies them to every palette the slot's `item_roles` gives the same role |

A free tile is blank, named by no entity's sprite set, and drawn by nothing: painting it changes none of the pictures the generator builds (level, HUD, sprites). `gp_mcp::resize` finds them by painting the candidates and building the art again, halving the set that changed something, so a sheet with a few used blank tiles (a sky tile) takes a few builds. Resize takes a sprite of one frame, without `cells` or clip frames of tiles, for now.

## Checks

Every edit runs these and returns them all:

| Check | What it measures | Effect |
|---|---|---|
| Readiness | 'What can be edited', above | Refuses |
| Ops | Each op's arguments against the picture | Refuses, naming the frame and op |
| `body.hitbox` | Drawn pixels outside the hitbox, before and after | Warns past 8 more: a player judges hits by the art |
| `body.stands` | Whether art that stood on the bottom row still does | Warns: it would float |
| Tiles | Blank tiles and tiles that repeat another, flipped or not | Advice |
| Critic | The consistency critic ([host](host.md#critic)) | `advise` by default, `enforce` with `min_score` refuses |
| `context.readability` | The sprite's colors against the backgrounds it was drawn over on every recorded frame | Warns when 5 points less of those backgrounds read at WCAG contrast 3 or more, or under half do |
| Placement | `assets_place`'s: packed, every baseline's state digest the same, the model checked | Refuses and puts everything back |

Readability is measured from the model's replay of the recorded input (`gp_mcp::scene`): for each frame the entity is drawn, the level picture's colors under its drawn pixels, weighted by how often, against each of its colors that is at least a tenth of its ink. On Moss Hop the player is drawn on 1,200 frames, 96.5% over the sky.

## Previews in context

`gp_mcp::scene` replays the source's baselines in the interpreter and draws them on the CPU from the presentation the generator compiles (`gp_gen_bevy::present`, the same pictures `gp-present` names for the Bevy game and the web preview), once with the copy's assets and once with the edit. Presentation never moves the simulation, so one replay serves both. For each pose the baselines show the entity in (machine state, standing or rising or falling, moving or still, facing) it takes a 64 by 64 crop around the entity at the pose's first frame and at its first moment (an event that touches it, such as `player.died`), before beside after, as one PNG. On Moss Hop that is 12 shots over the five baselines. Level color cycles and the HUD aren't drawn in the crops.

Each edit also gets the close-up preview: before, after, after facing left (when the sprite flips) and the changed pixels, in the sprite's own colors at 8x; an edit of several frames gets a row a frame instead, before beside after, so a clip is seen together. Both are written to `<copy>/previews/edit-<seq>.png` and `edit-<seq>-context.png` and returned as base64.

## Proposals, decisions and history

With `place: false` an edit is a proposal: checked and previewed, kept in the history, and written only when the person accepts it ([On the page](#on-the-page)). With `place: true` (the default) it's written at once, and it may still be accepted (kept) or rejected (undone). `assets_edit` refuses to accept or reject a proposal, or an edit the person decided: only the person answers those, on the card or with `gp assets proposal accept`, `reject` and `undo`, which `gp_run` refuses to an agent. The agent may still accept or reject an edit it placed itself that the person hasn't decided, and undo and redo work as before.

| Status | Meaning |
|---|---|
| `proposed` | Shown, not written; the person's accept places it with every check |
| `placed` | Written by the agent |
| `accepted` | Kept: a proposal by the person, a placed edit by the person or the agent (`decided_by`) |
| `rejected` | A proposal dropped, or a placed edit undone on a person's say |
| `undone` | Undone; redo puts it back |

The history is in the working copy (`gp_mcp::edit_log`). Placed edits are in `asset-edits.jsonl` and undone ones in `asset-redo.jsonl`. Proposals and rejections are in `asset-proposals/<seq>.json`. Each edit keeps what it changed: every 8 by 8 tile before and after as 64 hex digits (an arcade board sprite as 1024), the slot's palettes, or for a resize the sprite's tile list and width ([the art history](formats/art-history.md)). So:

- undo goes back any number of steps, the newest edit first, and the newest puts the slot's state back exactly (status, origin, license);
- `undo` with `seq` undoes an older edit alone when no later edit touched its tiles;
- `redo` puts undone edits back; a new edit empties the redo list;
- every write checks that the tiles are what the edit left (or found), and refuses rather than lose a change made since by another tool or by hand.

`assets_history` lists every edit with its status and who decided, and `waiting` lists open proposals: that's how an agent sees what a person decided.

## Level tiles

A level tile kind is an item of a metatiles slot: a 16 by 16 cell of four tiles, named by the asset map (`names`, the names the lifted metatile files carry and point and tell shows), with the slot's `item_roles` as its role. `assets_edit` with `tile` instead of `entity` edits one (`gp_mcp::tile_edit`). `tile` takes the kind's name (`stone block`), `<slot>[<index>]` (`metatiles[3]`) or its four tiles in hex (`05060708`).

| Part | For a tile kind |
|---|---|
| Picture | Its four tiles laid out 2 by 2, from the level tiles slot (the only tiles slot, or the one no sprite draws from; `slot` picks another) |
| Ops | `recolor`, `flip`, `mirror`, `shift`, `outline`, `fill`, `pixels`, and `pictures` (key `base`). `copy`, `load`, `match_style`, `tween` and `resize` read or change an entity's sprite and are refused |
| `palette_ops` | On the background palette the level picture draws the kind in: palette 0 for a grid level, as `gp_gen_bevy::present` draws every grid cell. `same_role` applies them to every palette the palettes slot gives the same role (`terrain` on Moss Hop: background palettes 0 to 3) |
| Readiness | The asset map names it, its tiles are in the level tiles slot, the palettes slot has its palette, a level's grid draws it, and the recorded baselines show it on screen. A missing item refuses with `stage: "model"` and how to fill it |
| Shared tiles | A kind that draws one tile twice must draw it alike after the ops (`stage: "frames"`). `also_changes` names the other tile kinds, and `also_changes_sprites` the sprites, that draw a changed tile. For palette ops, `also_changes` names the kinds drawn in a changed palette that use a changed color (the sky, drawn only in the backdrop, isn't changed by a new face color) |
| Previews | The close-up (before, after, changed, in the kind's colors) and, from `gp_mcp::scene::tile_in_context`, the baselines replayed: each cell of the kind is cropped (64 by 64 around it) on the first recorded frame the camera shows it whole, before beside after, up to 3 a baseline and 12 in all. `context.sprites_over_tile` measures the sprites drawn over the kind against it, before and after, as `context.readability` does for a sprite |
| History | Entries in the copy's history with `tile` set and no entity: proposals, accept, reject, undo and redo work as for sprites |

Nothing in it knows a game: every model whose metatiles slot comes from an asset map has its tile kinds, found from the map's names, the slot's roles and the levels' grids. On Moss Hop, "make it orange" on the stone block is a palette op, `color` index 3 to `$27`: the stone's face and the soil's specks (the same color of the shared terrain palette) turn orange in six crops of the five baselines, and accept places it with every state digest the same (game-printer's `crates/gp-cli/tests/point.rs`, not carried here). Making the stone alone orange needs it in a palette of its own, which needs per-cell background palettes in the model (open-questions rows).

## On the page

An `assets_edit` proposal (`place: false`, or an external editor's save through `gp assets open`) on a copy under `$GP_HOME/work/mcp/` is a card on the person's page (`mechanic/sprite-card.js`, issue #4), on an NES copy's tiles and an arcade copy's board sprites alike. The card is under the status line on every tab and on the copy's journey page, the oldest open one with how many wait ("1 of 2 waiting"). It shows the edit's own pictures with square pixels: the sprite before and after close up (`previews/edit-SEQ.png`) and, when the edit has them, the recorded frames before and after (`edit-SEQ-context.png`). It also has the agent's words (the edit's `prompt`), a box "tell your agent what to change", ACCEPT and REJECT, and PUT THE OLD ONE BACK after an ACCEPT.

| Step | Who | What happens |
|---|---|---|
| Propose | The agent | `assets_edit` with `place: false`, or a save in a pixel editor; the reply's `note` says the card waits |
| Show | The page | `next` lists each open proposal of every copy in `waits_for_yes` with `what: "sprite"` (its `sprite` the proposal, with `place` and `of`), and the copy's journey lists its own |
| Decide | The person | On the card, or in their terminal with `gp assets proposal accept ID`, `reject ID --words S` and `undo ID` (`gp assets proposal list` lists them; `gp_run` refuses the three, and `assets_edit` refuses to accept or reject a proposal). ACCEPT is the art history's accept with `decided_by` `person`: placed with every check, or refused in words when the art changed since. REJECT drops it. PUT THE OLD ONE BACK rejects the placed edit, which undoes it when no later edit changed its tiles. The history (`asset-proposals/`, `asset-edits.jsonl`) holds the answer as it does an agent's |
| Hear | The agent | `assets_history` gives `answers`: each answer the person gave on the page, with what they wrote, which marks it read; until then `next` lists it, so `next` with `wait_s` returns when the person answers |

The person's words and whether the agent has read them are kept beside the history, one file an answer, at `$GP_HOME/work/proposals/sprites/COPY-SEQ.json` (`gp_mcp::sprite_proposal`). gp serve's routes are `GET /api/sprites` (`{open, recent}`), `GET /api/sprites/ID/edit.png` and `context.png`, and `POST /api/sprites/ID` with `{answer: accept|reject|undo, words?}`, where ID is `COPY-SEQ`. When the person answers in the talk instead, the agent tells them to press the card's button; it can't answer for them. Until 2026-11-13 an edit could be told to a capture run (`run`) and accepted in the old editor's game view; nothing made a capture run any more, so that road was removed (the owner approved it on 2026-11-13).

A picture of a large-art theme does go through the page: see the next section.

## A picture of your own theme

The large-art themes the page plays (`theme/look.txt`, `theme/art.txt` and PNGs, lex00/turnips is the pattern) change one picture at a time through `gp theme picture` (crates/gp-cli/src/theme_picture.rs, issue #4). The agent proposes, the person decides on their page, and nothing in the theme changes before their ACCEPT.

| Step | Who | What happens |
|---|---|---|
| Propose | The agent, `gp theme picture propose DIR --game GAME --picture art/hero.png --png NEW.png --why "..."` through `gp_run` | Refused in words, and never shown to the person, unless: DIR is the person's own copy (a theme folder under `$GP_HOME/work/themes/`); the picture is one `art.txt` names (by its path, or the game picture a `pic` line draws with it) and is in the theme already; the new file is a PNG of at most 4096 pixels a side whose shape is within 15% of every rule that draws it; and `gp theme look check` on a copy of the theme with the picture in place finds no problem the theme didn't have, the ROM-data check among them. A proposal that passes is kept under `$GP_HOME/work/proposals/pictures/ID/` (`proposal.json`, `new.png`, and `old.png`, the picture as it was) |
| Show | The page | The oldest open proposal is a card under the status line on every tab, with how many wait ("1 of 2 waiting"): the picture the theme has now beside the new one, each enlarged with square pixels and at game size (the rule's size in game pixels), on a checkerboard so what is see-through shows, the agent's sentence, a box "tell your agent what to change", ACCEPT and REJECT. THEMES lists every open proposal and the last answered |
| Decide | The person | ACCEPT writes the new picture over the theme's (refused, and the proposal marked not used, when the file changed since it was proposed). REJECT keeps the theme and records the words. After an ACCEPT, PUT THE OLD ONE BACK writes the old picture back (one level). In a terminal, `gp theme picture accept`, `reject --words` and `undo` do the same; `gp_run` refuses those three, so only the person decides |
| Hear | The agent | `next` lists an open proposal and an answer the agent hasn't read in `waits_for_yes` (`what: "picture"`), so `next` with `wait_s` returns when the person answers; `gp theme picture list` gives the answers with the person's words and marks them read |

Not built yet: the picture placed on a still of the game (the stage drawn by tile kind, or with the lifted art when a ROM is here); the card shows the pictures on their own. A proposal is a whole PNG the agent wrote; the deterministic operations above (`recolor`, `flip` and the rest) work on a model's tiles, not on a theme's PNGs.

## External editors

`gp assets open <copy-or-template> <entity> --editor pixelorama` is the round trip with a pixel editor:

| Step | What happens |
|---|---|
| Copy | A working copy folder, or a template forked into `$GP_HOME/work/mcp/` (gp mcp's, so the agent sees the same copy) |
| Export | The sprite set through `gp-imagefmt` in the layout the image format config resolves ([image config](formats/image-config.md): defaults, `$GP_HOME/config/image.toml`, `gp.toml`, template and entity overrides, `--config`), into `<copy>/exchange/<entity>/`: `sheet.png` (or a PNG a frame), Pixelorama's `palette.json` (`--editor pixelorama`) or the config's palette file, `sprites.json`, and `gp-edit.json` (the copy, entity and slot, and each frame's name with its frame id) |
| Launch | The editor on the sheet, when it's installed: `GP_PIXELORAMA` (or `GP_LIBRESPRITE`), the PATH, then the usual application folders. Nothing is downloaded; when it isn't found the command says so and keeps watching |
| Watch | The images' digest every `--poll-ms` (500): Pixelorama doesn't reload a file changed on disk, so the person saves PNGs over the exported ones |
| On save | `gp-imagefmt`'s import checks (every pixel a color of its tile's palette, nothing outside the tiles, the tile budget), then `assets_edit` with the frames as `pictures` and every edit check, as a proposal in the copy's history (or placed at once with `--place`) |
| Accept | Placed with `origin = "uploaded"` and the editor in the slot's `generator` (`pixelorama via gp assets open (...)`), undoable like any edit |

`--editor libresprite` writes Aseprite's sheet JSON beside the sheet, which LibreSprite and Aseprite import. `--once` stops after the first save and `--timeout` after a time, for scripts. An agent does the same over MCP: `assets_export` writes the set by the config (`set.dir`), and `assets_edit` with `import` brings the folder back. Pixelorama's `.pxo` project isn't written or read yet (an open question).

### Arcade boards

On a copy whose lift is an arcade one (`gp_assets::arcade`, docs/formats/board-art.md), `gp assets open`, `assets_export` and `assets_edit`'s `import` take the entity's board sprites (`gp_mcp::board_trip`). A Midway MCR sprite is one 32 by 32 picture of 16 values in the `board-sprites` slot's sheet, so it isn't cut into tiles or laid out by the image config:

| Step | What happens |
|---|---|
| Export | One PNG for each picture code the entity's `board_sprite` draws, `ENTITY-CODE.png` (`tip-4d.png`): 8-bit indexed in the 16 colours of its colour bits from the level's lifted `board-palette` (the program's writes over it aren't applied), values 0 and 8 clear since the board draws nothing where a value's low three bits are 0. A fixed `picture = { number = N }` is one code; a picture read from the body's words takes the codes `gp theme look sheet` saw (its `names.toml`), and without that sheet the export says to run it. Beside them `ENTITY.gpl` and `gp-edit.json`, whose frame ids are `board-sprite.CODE`. A folder inside a git work tree is refused: the pictures are the person's ROM's |
| On save | Each PNG is read back held to 32 by 32 and values 0 to 15 (an indexed PNG's indices, or an RGB(A) one's colours matched exactly to the exported 16), and compared with the lift |
| Proposal | A save that changed pixels is kept in the art history as the NES trip's is: proposed, or placed with `--place` (`place: true`). Each changed sprite is a `tiles` item, its index in the slot's sheet with its 1024 values before and after in hex digits ([the art history](formats/art-history.md)), and the slot's state comes from the slot catalog reading the arcade map (`catalog::arcade_slot_assets`). The reply counts the pixels of each code that changed, and `edit-SEQ.png` in the previews shows each sprite before and after in its colours. A save that changed nothing is reported and kept nowhere |
| Accept, undo, redo, reject | Write the sprites into the copy's own lifted folder (`<copy>/assets/`, made from the lift on first use, map last), never the shared lift, once the sheet holds what the edit found (or left); the slot becomes `replaced` from `uploaded` with the editor as its generator, and undo of the newest puts its state back. `gp_assets::arcade::unpack` puts that folder back into the chips' layout. No baseline is run: the board draws these sprites, and a slot the simulation reads is refused |

The board's tiles (`board-tiles`, `board-font`) have no editor trip yet: no entity draws with them, and `assets_edit`'s `tile` names an NES level tile kind.

On a scratch copy of Tapper on 2026-11-14, lifted with its own map: `gp assets open COPY tip --editor png --no-launch --once` wrote `tip-4d.png`, `tip.gpl` and `gp-edit.json`; four pixels changed to values 12 to 15 came back as "save 1: edit 1 proposed (4 pixels)", its rows holding `c` to `f`. `assets_edit` with `accept: 1` wrote `sprites.png` in the copy's own folder, whose sprite 0x4d differs from the lift's in exactly those four pixels and nowhere else, and `surface_describe` listed `board_sprites` as replaced from uploaded, every one of the copy's 56 slots with its arcade format and shape.

## A demo

```sh
just setup
# Moss Hop's ROM and lift, and an agent's MCP server:
cargo test -p gp-cli --test mcp -- a_save_in_a_pixel_editor_comes_back_as_a_proposal  # a fake editor process
GP_EDIT_PREVIEW=/tmp cargo test -p gp-cli --test mcp -- an_agent_edits_the_player_sprite_from_its_recordings  # writes the preview
```

By hand, connect an agent to `gp mcp`, fork Moss Hop (`template_fork`) and ask it to "make his moss cap the color of his eye". It calls `assets_edit` with `ops: [{op: recolor, pairs: [[2,3]], region: {x:0,y:0,w:16,h:5}}]` and `place: false`, and the proposal waits on the person's page as a card; on their ACCEPT, `assets_history` shows it accepted by the person. To draw by hand instead: `gp assets open mosshop player` (with Pixelorama installed), save in Pixelorama, and the save comes back as a proposal the same way.

## Phases

| Phase | Builds | State |
|---|---|---|
| 1 | The agent command surface and previews with accept and reject; undo and redo; sprite sets of tile frames; palette ops, match style, tween, resize; readability against recorded backgrounds | Built (2026-10-29) |
| 2 | Whole sprite sets: frames as piece layouts, sprite-set rows resolved, a clip's frames edited together and held consistent, a row a frame in the close-up; proposals in the game view | Built (2026-10-29) |
| 3 | `gp assets open`: export by the image format config, launch Pixelorama if installed, watch, check and preview each save, accept with provenance; LibreSprite through Aseprite's sheet JSON; `assets_export` and `assets_edit import` for agents | Built (2026-10-29), without `.pxo` |
| 4 | The same ops on a theme's `.pix` restyles | Milestone 4 |
| 5 | A free local image model behind `Generator` for redraws and in-betweens; agent-driven external editors | Later |

## Open questions

Filed in game-printer's docs/open-questions.md (history, not carried here; rows for M3 from m3/sprite-editor) are readability against the original's recorded frames and `gp gaps drawn` offsets, resize of laid-out and animated sprites, and color index names. They also cover a proposal's in-context preview in the interpreter preview itself and frames with pieces of several palettes. The list ends with `.pxo` and trying the edit tools on Super Mario Bros.' captured sets in the ROM tier.
