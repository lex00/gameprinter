# The reference game

Moss Hop is a small NES platformer written for Game Printer. The project owns it outright: its code, graphics and level are original, and it's licensed with the repo under Apache-2.0. Since no commercial ROM is involved, every check that runs on it runs in `just check`. Its game model, written by hand from this source and checked against its baselines, is in `templates/mosshop/`. It came from game-printer's docs/reference-game.md; the game, its baselines and its model came over unchanged, and the checks on other platforms are in [platform-checks.md](platform-checks.md).

## Build it

```sh
just mosshop    # games/mosshop/ -> target/mosshop/mosshop.nes
```

`scripts/mosshop/build.sh` assembles `games/mosshop/mosshop.s` with `ca65` and links it with `ld65` and `games/mosshop/nrom.cfg`. It also writes `mosshop.dbg` (ld65's debug file) and `mosshop.labels` (VICE label format) next to the ROM. `just check` runs the same build before checking the game's baselines.

| Needs | Where it comes from |
|---|---|
| cc65 2.19 (`ca65`, `ld65`) | `brew install cc65` on a Mac, or `apt install cc65` on Debian or Ubuntu |

cc65 2.19 reports itself as `ca65 V2.18`, both Homebrew's build and Debian's. The script prints a note when the version line differs, then builds anyway. The note can't tell 2.18 from 2.19, which print the same line: the baselines pin the ROM's SHA-256, so a toolchain that assembles different bytes fails `gp baseline check` with a message saying so. Without `ca65` or `ld65` the script exits 2 with the install command for your system.

The ROM image is built at check time and never committed (see the decision below).

## How it plays

The player is a round moss creature. The level is two screens (512 pixels) wide and scrolls horizontally. It ends at a burrow, a moss mound with a doorway. On the way are two slugs and a pit, and a ? block and five coins to take. The counters along the top show the coins taken and the lives left.

| Mechanic | Rule |
|---|---|
| Walk | Left or Right moves 1 pixel per frame |
| Run | Holding B makes it 2 pixels per frame |
| Jump | A, newly pressed while standing, sets the vertical speed to -5 pixels per frame. Holding A longer changes nothing |
| Gravity | Adds 1/4 pixel per frame to the vertical speed every frame, up to 4 pixels per frame downwards |
| Jump height | 47.5 pixels: from `PLAYER_Y` 176 to 128.5. In `jump-height.toml`, `ON_GROUND` drops to 0 at frame 61, the peak is reached at frame 79 (18 frames later), and the player lands at frame 100 (39 frames after frame 61) |
| Blocks | Stone blocks, the ? block and the ground are solid from every side; the player stops against them and lands on them |
| Head | While rising, the cell at the head point (`PLAYER_X` + 8, `PLAYER_Y`) decides: a solid cell sets the vertical speed to 0 where the player is. The ? block in it becomes a used block and gives a coin |
| Coins | A coin in the cell at the pickup point (`PLAYER_X` + 8, `PLAYER_Y` + 8) is taken: the cell becomes sky and `COINS` goes up by 1 |
| Enemy | Slugs crawl 1 pixel every other frame, back and forth along a fixed stretch of ground. There are two, of one kind |
| Stomp | Touching a walking slug while falling, with `PLAYER_Y` below 172, squashes it and bounces the player up at -3 pixels per frame. That means the player's bottom row is in the slug's top 7 rows (180 to 186). A squashed slug lies flat for 30 frames, touching nothing, then goes |
| Death | Touching a walking slug any other way, or falling into the pit (`PLAYER_Y` 208 or more), kills the player. The player turns upside down, hops up at -4 pixels per frame and falls through everything off the screen, while the slugs, their touches and the camera stop. After 90 frames a life goes and the level starts over with both slugs and every coin and block back; `DEATHS`, `STOMPS` and `COINS` keep counting |
| Lives | Three tries: `LIVES` starts at 2. With none left the game is over (`STATE` 3) for 120 frames, then a new game starts with every counter at 0 |
| Goal | Reaching `PLAYER_X` 468, which puts the player in the burrow's doorway, clears the level (`STATE` 2); the game then stops |
| Camera | Keeps the player 120 pixels from the left edge of the screen, within the level |

The game reads controller 1 once per frame in its main loop, so the only lag frames are the 5 at power-on, before the loop starts.

The level's cells live in RAM (`LEVEL_LEFT` and `LEVEL_RIGHT`, a page of 16 columns each), copied from ROM at each start. A cell that changes is queued (`REDRAW_*`) and drawn again in the next NMI.

The level, as `mosshop.s` writes it, has one character per 16x16 metatile. Rows 0 to 9 are empty sky.

| Character | Metatile |
|---|---|
| `.` | Sky |
| `M` | Moss over soil |
| `S` | Soil |
| `B` | Stone block |
| `?` | ? block (a used block, `u`, once bumped) |
| `o` | Coin |
| `(` and `)` | The burrow's left and right halves |

```
row 8   ....?...........................
row 9   .................ooo............
row 10  ............oo...BBB.....B......
row 11  ......B..................B...().
row 12  MMMMMMMMMMMM..MMMMMMMMMMMMMMMMMM
row 13  SSSSSSSSSSSS..SSSSSSSSSSSSSSSSSS
row 14  SSSSSSSSSSSS..SSSSSSSSSSSSSSSSSS
```

From the start at x 24: the ? block over x 64 to 79, a one-block step at x 96, the first slug crawling between x 128 and 176, a pit from x 192 to 223 with two coins over it, floating blocks with three coins on them at x 272 to 319 with the second slug crawling beneath them between x 256 and 352, a two-block wall at x 400, and the burrow from x 472 to 487.

## RAM map

Every variable has a fixed address in zero page. `games/mosshop/ram.inc` defines them for the assembler and `games/mosshop/labels.toml` lists them for tools: `gp labels`, selectors such as `ram[0x0012]` and the `label` of each baseline assertion. `just check` runs `gp labels check templates/mosshop/mapping.toml`, which lists any RAM address the state mapping reads that has no label. Not here yet: game-printer's test that `ram.inc` and `labels.toml` agree.

| Address | Label | Meaning |
|---|---|---|
| `$00` | `FRAMES` | NMI count, wraps at 256 |
| `$01` | `NMI_DONE` | Set by the NMI handler, cleared by the main loop |
| `$02` | `PAD` | Controller 1 this frame, A in bit 7, Right in bit 0 |
| `$03` | `PAD_PREV` | Controller 1 the frame before |
| `$04` | `PAD_NEW` | Buttons down this frame and up the frame before |
| `$05` | `STATE` | 0 playing, 1 dying, 2 level clear, 3 game over |
| `$06` | `TIMER` | Frames left in the dying or game over state |
| `$07` | `DEATHS` | Deaths since the game started |
| `$08` | `STOMPS` | Slugs stomped since the game started |
| `$09` | `COINS` | Coins taken since the game started |
| `$0A` | `LIVES` | Lives left after this one |
| `$10` | `PLAYER_X` | Player's left edge in the level, low byte |
| `$11` | `PLAYER_X_HI` | Its high byte, 0 or 1 |
| `$12` | `PLAYER_Y` | Player's top edge, pixels from the top of the screen (176 standing on the ground) |
| `$13` | `PLAYER_YSUB` | Fraction of `PLAYER_Y`, in 256ths |
| `$14` | `PLAYER_VY` | Vertical speed in whole pixels per frame, signed, down is positive |
| `$15` | `PLAYER_VYSUB` | Fraction of the speed, in 256ths |
| `$16` | `ON_GROUND` | 1 while standing on something solid |
| `$17` | `FACING` | 0 right, 1 left |
| `$18` | `JUMP_START_Y` | `PLAYER_Y` when the last jump started |
| `$19` | `PEAK_Y` | Smallest `PLAYER_Y` since the last jump started |
| `$1A` | `CAMERA_X` | Left edge of the screen in the level, low byte |
| `$1B` | `CAMERA_X_HI` | Its high byte |
| `$1C` | `PLAYER_Y_HI` | High byte of `PLAYER_Y`: 1 once a dying player falls below the screen |
| `$20-$21` | `ENEMY_X` | Each slug's left edge, low byte |
| `$22-$23` | `ENEMY_X_HI` | Its high byte |
| `$24-$25` | `ENEMY_DIR` | 0 moving right, 1 moving left |
| `$26-$27` | `ENEMY_STATE` | 0 gone, 1 walking, 2 squashed |
| `$28-$29` | `ENEMY_TIMER` | Frames left squashed |
| `$30-$46` | `TMP_*` | Scratch, valid only inside one routine |
| `$50-$68` | `REDRAW_*` | Up to 8 metatiles queued for the next NMI to draw |
| `$0200-$02FF` | `OAM` | Sprite table, copied to the PPU by DMA in every NMI |
| `$0300-$03EF` | `LEVEL_LEFT` | The level's cells, columns 0-15, 16 a row |
| `$03F0-$04DF` | `LEVEL_RIGHT` | Columns 16-31 |

Positions are in pixels. Player and slugs are 16x16 sprites with a 12-pixel-wide hitbox (from x + 2 to x + 13). A slug's hitbox is the bottom 12 rows of its sprite (from y 180 to 191), its eye stalks included.

## Baselines

Eight baselines in `baselines/mosshop/` script the game and commit what it does: the frame hash digest, the lag frame count and RAM values at chosen frames.

| Baseline | Run | What its assertions pin |
|---|---|---|
| `finish-level.toml` | Runs right with B held and jumps four times, landing on the floating blocks on the way | Three coins by frame 146, the stomp of the second slug at frame 169 and its going at 199, the level clear at frame 250, no deaths |
| `slug-death.toml` | Jumps the step, lands just behind the first slug and runs into it | The death at frame 74 (`PLAYER_VY` -4, the hop), the player below the screen by frame 130 (`PLAYER_Y_HI` 1), the restart 90 frames after the death with a life fewer |
| `stomp-then-pit.toml` | Stomps the second slug, then runs back left into the pit | The stomped slug, the coins and the death counted after the restart, the slug and a coin's cell back |
| `coins-and-pit.toml` | Stomps the first slug, takes the two coins over the pit and falls in | The squash at frame 105 for 30 frames, the coins at 109 and 125 and their cells emptied, the pit death at 139, the restart at 229 with the coins back and `COINS` kept |
| `bump-block.toml` | Walks under the ? block and jumps | At frame 56 the head point is in the block's cell: the rise stops, `COINS` is 1 and the cell is the used block |
| `game-over.toml` | Runs into the first slug three times | Deaths at 74, 234 and 394, game over at 484, a new game at 604 with three lives and every counter at 0 |
| `jump-height.toml` | One standing jump | Peak `PLAYER_Y` 128 with `PLAYER_YSUB` $80 at frame 79, landing at frame 100 |
| `walk-and-turn.toml` | Walks right without B, jumps, meets the ? block's side in the air, turns left and walks back | Walking at a pixel a frame (x 44 at frame 30), stopped at x 50 by the block, the turn at frame 42, x 10 at the end |

`gp baseline check --rom target/mosshop/mosshop.nes baselines/mosshop` checks them with the built ROM and no ROM folder (`--video` also runs each with video rendering on, and fails if its frame hashes change). A baseline whose `sha256` isn't the given ROM's can't be checked, so a change to the game's source can't make its checks pass by not running. After changing the game on purpose, record the baselines again: put the new SHA-256 in each, then take the frame hash digests and RAM values `gp baseline check` reports, after checking that they're what the change should do. The asset map, `assets/mosshop.toml`, and `templates/mosshop/template.toml` pin the same SHA-256, and `gp assets lift` and `gp template check` in `just check` fail until they're updated ([Asset lifting](assets.md), [Templates](templates.md)).

## Where it's checked

`just check` builds the game and runs, on Moss Hop: the baselines, the asset round trip (`gp assets lift` and `gp assets pack --check`), `gp model check` with the lifted assets, `gp model fmt --check`, `gp labels check` on its mapping, `gp interp compare` on each baseline, the browser player against the native one (`just play-web`), the mechanic page (`just mechanic-page`) and `gp template check`. `just validate-mosshop` holds the model to its contract against the baselines ([validation](validation.md)), and `just derive-mosshop` reads its facts back from the source ([derivation](derivation.md)).

game-printer's `just check-open-platforms` (Rosetta and wasmtime with `gp-wasm-check`) is `just check-wasm` and `just check-x86_64` here ([platform-checks.md](platform-checks.md)); its `just docker-check` in Linux containers didn't come. `gp input-log generate` turns a baseline into an input log to record and replay.

## Ground truth

Since the source is ours, `games/mosshop/ground-truth.toml` lists every routine and branch with its CPU address and the inputs each routine reads. It also tags each routine with its element and marks the routines that run only at power-on and the branch directions the level's data never takes. `scripts/mosshop/ground_truth.py` writes it from the source and ld65's debug file; with `--check` it fails when the file is stale. `gp model static target/mosshop/mosshop.nes --grade games/mosshop/static-key.toml` grades stage 0's harvest against it: the elements found and the routines placed, with the facts against the real model. `scripts/mosshop/measure_matrix.py` and `measure_grid.py` measured game-printer's `gp clips matrix` and `gp capture grid` against it; neither command is here yet.

## Decisions

| Question | Decision |
|---|---|
| Is the ROM image committed? | No. The pre-commit hook refuses any file with an NES header and `.gitignore` covers `*.nes`, and an exception for one file would weaken both. The ROM is 24KB and builds in well under a second, so `just check` builds it from source every run. The baselines' `sha256` is what pins it |
| How do checks find a ROM that isn't in `$GP_ROM_DIR`? | `gp baseline check --rom FILE` and the other commands' `--rom` take it by path. The ROM is built into `target/`, inside the work tree, where `$GP_ROM_DIR` may not point |
| Which assembler? | cc65 2.19 |
| What license? | Apache-2.0, like the rest of the repo, so templates and forks made from it can be shared |
