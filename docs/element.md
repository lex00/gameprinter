# Reading one element's code: gp element inputs

`gp element inputs` reads, from the original's own run, the routine that wrote a measure's byte on a clip's frame and every byte and branch its value depends on. It is steps 2 and 3 of game-printer's element-extraction skill, which comes to gameprinter once waves 3b and 4 have landed.

```sh
gp element inputs --run games/donkey-kong --clip dk.walk.001 --module ~/.gameprinter/work/modules/donkey-kong.wasm
gp element inputs --run games/donkey-kong --clip dk.walk.001 --frame 120 --measure 'entities.player[0].x'
gp element inputs --run games/donkey-kong --clip dk.double-barrel-jump.003 --frame 200 --byte '$0019'
```

`--run` is the game's package folder and `--clip` a clip of its corpus. In game-printer `--run` named a capture run, whose kept clips, background scores and bench the command read; gameprinter has no capture runs until wave 7 of the port, so it reads the game's package as `gp capture dense` and `gp capture carry` do (docs/capture.md).

What it needs to run the model, and when:

| Given | Runs |
|---|---|
| `--frame` and `--byte`, or `--frame` and `--measure` of a field (or of a body with `--slot`) | The original only, from the clip's start (the start cache keeps it) |
| No `--frame`, no measure, `--entity`, or a body's measure without `--slot` | Also the clip's lockstep straight through, as `gp clip score` runs it (with the original's draws when the mapping sets `score_draws`): its first wrong measures (`gp_interp::scenario::tracks`) and the original's slot of each body the lockstep pairs. A model with hooks needs `--module`; `--model`, `--assets` and `--rom` are `gp clip score`'s |

The output (JSON schema `gp element inputs 1`) is per byte: the instruction that last wrote it and its routine, the bytes the write's value came from through indexes and pointers, the branches that decided it, each with a role, the routines one level on that set those inputs earlier in the frame (`--depth`), and for each input an earlier frame left, the frame and routine that wrote it (`--no-back` leaves that out). A sentence sums it up ("move_enemies wrote ENEMY_X+x from ENEMY_X+x").

Names come from the game's `labels.toml` in its folder, or `--labels FILE` (docs/formats/labels.md, with wave 1's `gp labels`). Without one, every name is an address. Only routine and label names, addresses and values are printed: never code.

It reads the 6502's code, so an arcade romset is refused in words. Its arcade counterpart needs gp-arcade to log each memory read and write with the PC that made it, and a Z80 writer slice in place of `gp_nes::slice` (wave 6).

## On an arcade game (Z80)

`gp element inputs --run games/tapper --clip ID --frame N --byte ADDR` on an arcade game (port wave 6) slices the byte on the main Z80 (`crates/gp-cli/src/element_z80.rs`). The clip runs from power-on with gp-arcade's instruction trace and access log on over its own frames; on the frame the byte was last written, the slice walks back from the write through the registers the stored value came from, and the instructions that set them, up to 400 instructions. Each memory read on the way is an input, `value` or `index` (it set an address register the write or a later read went through), with its value and whether the frame wrote it first (`frame`) or it came from before (`before`); the conditional jumps run between the first input and the write are listed with the ways they went; the routine is the call or interrupt the write ran in. `--measure`, `--slot` and `--entity` need a lockstep against a model, which isn't run for an arcade game here yet, so `--byte` and `--frame` are required.

On Tapper's `tapper.score.001`, frame 300: "$1C02 wrote $E2FD at $1CDD (`ld (ix+$17),a`) from $E2FD" (a position stepped from itself, $69 to $6B), and "$00D8 wrote $E403 at $0119 (`ld (hl),a`) from $E403" ($00D8 is the CTC's frame interrupt handler; $E403 is the frame counter `gp track` found).
