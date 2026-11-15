# Reading one element's code: gp element inputs

`gp element inputs` reads, from the original's own run, the routine that wrote a measure's byte on a clip's frame and every byte and branch its value depends on. It is steps 2 and 3 of the element-extraction skill (`.claude/skills/element-extraction/`).

```sh
gp element inputs --run games/donkey-kong --clip dk.walk.001 --module ~/.gameprinter/work/modules/donkey-kong.wasm
gp element inputs --run games/donkey-kong --clip dk.walk.001 --frame 120 --measure 'entities.player[0].x'
gp element inputs --run games/donkey-kong --clip dk.double-barrel-jump.003 --frame 200 --byte '$0019'
```

`--run` is the game's package folder and `--clip` a clip of its corpus. In game-printer `--run` named a capture run, whose kept clips, background scores and bench the command read; here it reads the game's package as `gp capture dense` and `gp capture carry` do (docs/capture.md).

What it needs to run the model, and when:

| Given | Runs |
|---|---|
| `--frame` and `--byte`, or `--frame` and `--measure` of a field (or of a body with `--slot`) | The original only, from the clip's start (the start cache keeps it) |
| No `--frame`, no measure, `--entity`, or a body's measure without `--slot` | Also the clip's lockstep straight through, as `gp clip score` runs it (with the original's draws when the mapping sets `score_draws`): its first wrong measures (`gp_interp::scenario::tracks`) and the original's slot of each body the lockstep pairs. A model with hooks needs `--module`; `--model`, `--assets` and `--rom` are `gp clip score`'s |

The output (JSON schema `gp element inputs 1`) is per byte: the instruction that last wrote it and its routine, the bytes the write's value came from through indexes and pointers, the branches that decided it, each with a role, the routines one level on that set those inputs earlier in the frame (`--depth`), and for each input an earlier frame left, the frame and routine that wrote it (`--no-back` leaves that out). A sentence sums it up ("move_enemies wrote ENEMY_X+x from ENEMY_X+x").

Names come from the game's `labels.toml` in its folder, or `--labels FILE` (docs/formats/labels.md, with wave 1's `gp labels`). Without one, every name is an address. Only routine and label names, addresses and values are printed: never code.

On an arcade romset it reads the main Z80's code instead (below).

## On an arcade game (Z80)

`gp element inputs --run games/tapper --clip ID --frame N --byte ADDR` on an arcade game (port wave 6) slices the byte on the main Z80 (`crates/gp-cli/src/element_z80.rs`). The clip runs from power-on with gp-arcade's instruction trace and access log on over its own frames; on the frame the byte was last written, the slice walks back from the write through the registers the stored value came from, and the instructions that set them, up to 400 instructions. Each memory read on the way is an input, `value` or `index` (it set an address register the write or a later read went through), with its value and whether the frame wrote it first (`frame`) or it came from before (`before`); the conditional jumps run between the first input and the write are listed with the ways they went; the routine is the call or interrupt the write ran in. `--measure`, `--slot` and `--entity` need a lockstep against a model, which isn't run for an arcade game here yet, so `--byte` and `--frame` are required.

On Tapper's `tapper.score.001`, frame 300: "$1C02 wrote $E2FD at $1CDD (`ld (ix+$17),a`) from $E2FD" (a position stepped from itself, $69 to $6B), and "$00D8 wrote $E403 at $0119 (`ld (hl),a`) from $E403" ($00D8 is the CTC's frame interrupt handler; $E403 is the frame counter `gp track` found).

## What is at a spot: gp element explain

`gp element explain` answers a person's "what is this?" (issue #10). The person pauses the game view on REFINE, clicks a spot or drags a box on either game and presses "what is this?"; the question reaches the agent as their turn in the open mechanic's talk, with the call that answers it, and the agent's reply shows under the box. The same command answers an agent's own question about a spot.

```sh
gp element explain --run games/donkey-kong --clip dk.walk.001 --frame 120 --at 100,180
gp element explain --run games/donkey-kong --clip dk.walk.001 --log-frame 412 --box 96,120,16,16 --side model
gp element explain --run games/tapper --clip tapper.l1.001 --frame 200 --at 300,260 --no-model --json
```

The spot is in screen pixels of the picture as drawn: the NES's is 256x240, Tapper's board 512x480 (two pixels a game unit). `--frame` counts from the clip's start, as `gp clip score` and `gp element inputs` do; `--log-frame` counts from power-on, as the page's game view and a point-and-tell turn do. `--side` says which game's picture the person pointed at; both are read either way. Only corpus clips are read so far: a free play clip or a store recording gets no call, and the turn says why.

What it says, each part with where it came from (`from` in the JSON, schema `gp element explain 1`):

| Part | From |
|---|---|
| What is drawn there: the sprites under the spot (their slot in the sprite table, picture number, palette and place) and the background tile at its middle (its cell, picture number, and the asset map's name for its metatile when the model has lifted one), and the HUD fields the spot covers | The original run to the frame: gp-nes from the clip's start (the start cache), OAM as the frame ended and the sprite table's page from the frame's write to $4014, the nametables at the frame's scroll; gp-arcade from power-on, sprite RAM at $E800 and video RAM at $F000 |
| Which game object it is, with the mapping's field values on the frame | The state mapping: the sprite's x and y bytes, or the bytes the code that wrote them read, followed back up to three writes, set against the mapping's fields and slot fields (a slot's kind told by its ID byte; a group the mapping calls vacant on the frame passed over). When no code read leads to a named byte, a slot whose block of bytes (its stride) holds the sprite's byte answers, marked `found_by: place` |
| What drew it | `gp element inputs --byte` on the sprite's x or y byte, whichever was written last: the routine and instruction, and the sentence |
| What moves it | `gp element inputs --byte` on the object's x and y bytes: the routines that wrote them and every input that code read |
| What the model has there, and whether the two agree | The clip's lockstep straight through with both pictures, as `gp clip score` and REVIEW REBUILD run it: the things the model's picture draws in the spot, how many pixels of the two pictures differ there, and the object's measures on the frame in both games, each within its tolerance or not |
| What the tools could not determine | Said plainly in `unknown`: an object the mapping has no bytes for, a byte that came with the clip's start, the NES's nametable writes (made through $2007, which the slice doesn't follow to a cell), no labels |

It takes a few seconds on a clip whose score tape and start are kept; the output ends with how long the original, the code and the model took. `--no-model` leaves out the model's lockstep, the slowest part on a long clip. The first run after a new gp builds the game's module once.

Through MCP it runs freely (`gp_run element explain`, a read). A turn that asks what something is shows in `gp mechanic show` with "asks what this is: answer from" and the call to run.
