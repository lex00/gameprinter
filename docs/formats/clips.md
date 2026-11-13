# Clip set format

A clip set is a list of clips as recipes: each clip's start and presses, which anyone can make again from the ROM, with the curve the original must give on them. It holds no snapshot, no frames and no other game data. It came from game-printer's docs/formats/clips.md. There a template kept its clips in `clips.toml`; here a game keeps them in its corpus plan, `corpus.toml` with its presses in `inputs/` ([scoring corpus format](scoring-corpus.md)), which holds no curves. The clip set format is still what the search commands write and what several readers take, and a game's `corpus.toml` can be given wherever a clip set is asked for.

`crates/gp-cli/src/clipset.rs` is the code that reads and makes them, and its JSON Schema is `schema/clips.json`, which `gp self schemas` writes from the Rust types. This page describes format `0.5.0`, which added controller 2's presses, spans with the `p2:` prefix. 0.4.0 had added `[import]`, 0.3.0 the `dense` clip kind and its `coverage` hash, and 0.2.0 the recipe and content hashes. A reader takes the same major and minor version, and older ones, which each version only adds to. Choosing clips again never changes the model's hash, since a model's text doesn't include its clips.

| Reads a clip set | How |
|---|---|
| `gp code listing ROM --clips SET` | Runs each clip from power-on and counts the instructions and branch ways ([code reading](../code-reading.md)) |
| `gp reach ROM --clip SET`, `gp model static ROM --clips SET` | Run the clips after stage 0's own starts |
| `gp explore ROM --start SET` | Starts its search from the end of each clip |
| `gp capture carry GAME`, `gp corpus score GAME`, `gp game score` | Read the game's own clips: its corpus plan, else its `clips.toml` |
| `gp corpus init`, `gp corpus bump` | Also read the older form, a clip set with a corpus manifest beside it; nothing in the games checkout uses it |

| Writes a clip set | Where |
|---|---|
| `gp explore --write DIR` | `DIR/clips.toml`, the kept clips from power-on, with each start's presses |
| `gp capture dense GAME --into DIR` | A scratch folder, never the game's ([dense clips](../research/dense-clips.md)) |

A clip that plays a sealed run is left out by every reader, which says how many it left out ([sealed runs](../sealed.md)).

## Example

A made-up set with one clip from a save point:

```toml
[clipset]
format = "0.5.0"
game = "my-game"
rom = "<the ROM's headerless SHA-256>"
mapping = "22"

[[clips]]
id = "explore.0001"
name = "Searched presses, 120 frames, from the end of walk 001"
source = "explore"
presses_file = "clips/explore.0001.presses"
frames = 120

[clips.start]
presses_from_power_on = "clips/explore.0001.start.presses"
frame = 4770
context = "save_point"
```

## `[clipset]`

| Key | Required | What |
|---|---|---|
| `format` | yes | `"0.5.0"` |
| `game` | no | The game's ID |
| `rom` | no | The ROM's headerless SHA-256, as the game's `roms.toml` gives it. A reader refuses to make the clips on another ROM |
| `generator` | no | What made the clips |
| `mapping` | no | The state mapping's `schema` the curves were measured with. Under another one a curve is measured again, not checked |
| `model`, `run` | no | The model hash and run the set was chosen on, for a reader; nothing checks them |

`[recordings.ID]`, `[[mutant]]` and `[import]` are read and written back unchanged, so a set this gp rewrites keeps them, but nothing here uses them: game-printer's capture runs named recordings there and its prune read the mutants. `[import]` now lives in the corpus plan ([scoring corpus format](scoring-corpus.md#import)).

## `[[clips]]`

| Key | Required | What |
|---|---|---|
| `id` | yes | The clip's ID, unique in the file |
| `name` | yes | In game words |
| `source` | no | `generated` (the default), `played` (a person's, from a save point), `aimed`, `dense` (chosen for the original's code it runs) or `explore` (found by `gp explore`, which searches for presses that run code no run has run, with no model) |
| `presses` | no | Button spans from the start, frames counted from 0: `right+b@0-59`. A span with the `p2:` prefix is controller 2's: `p2:start@60-60` |
| `presses_file` | no | A `.presses` file in place of `presses`, for a long list |
| `frames` | yes | Frames the clip runs, presses and settle |
| `curve` | no | The BLAKE3 of the original's mapped measures on every frame of the clip (each measure's path and its values, a line each) |
| `coverage` | for `dense` | The BLAKE3 of the original's code map over the clip. It depends on the ROM, the mapping and the presses, not on the model |
| `holds`, `guards`, `aims`, `found`, `requires` | no | The parts it exercises, the mutants it catches, the mutants an aimed clip is aimed at, the fixes found on it, and the clips a queue needs to pass first. Kept for a reader; nothing here acts on them |
| `why` | no | Why it is kept or made, in game words |
| `start` | yes | Where it starts, below |

### `start`

| Key | What |
|---|---|
| `presses_from_power_on` | A `.presses` file of the input from power-on: a save point's |
| `recording` | A recording's ID; the start is a frame of its input. This gp reads such a clip but can't make it: a game's package holds no recordings |
| `frame` | The original's frame. It must read the controller once the mapping's start holds, except at the end of a save point's presses (below) |
| `model_frame` | The model's frame there, as the lockstep counts it; checked when the start is made |
| `context`, `level` | The start's kind (`level_start`, `pit`, `wall`, `save_point`, a corpus context) and level, for names |

Exactly one of `recording` and `presses_from_power_on` is given. Every file path is relative to the clip set's folder and stays inside it.

The tools that play a clip keep the console a few frames before its start as a snapshot outside the repo, in `$GP_HOME/work/snapshots/starts/`, keyed by the ROM and the input to there, so clips with the same start presses share one and a clip plays only its own frames after the first time. The set holds only the recipe either way.

A dense clip is refused by the commands that make or score clips: this gp can't make one yet.

## `.presses` files

```text
gp-presses 0
frames 4772
right@120-180
right+b@181-240
```

The first line is the format, the second the frame count, then one span a line, controller 2's with the `p2:` prefix. An arcade game's files say which set and buttons they're for on their third line, `arcade tapper CSRLDUT`, and name the buttons `coin`, `start1`, `up`, `down`, `left`, `right` and `tap`. A save point's presses from power-on are the player's own, as a baseline's are, and are committed; the snapshot they lead to is not.

A save point's start whose frame doesn't read the controller, where its presses from power-on end, goes on into the clip's own presses to the first frame that does, and the clip starts there. A clip from power-on is one: frame 0 reads nothing, and Donkey Kong first reads the controller a few frames in. The frames before the start move from the front of the clip's presses to the end of the start's input, so the original plays the same input to the same last frame, as the mechanic page scores such a clip (`clipset::take_on`). A clip whose frames end first can't be made.

Not here yet: `gp capture generate` and `gp capture prune`, which made a set's clips in a capture run and chose the kept set by the mutants it catches. `gp capture regenerate`, which makes the clips again and checks each curve, is here as `gp corpus score GAME --run RUN`. `gp corpus score` and `gp game score` make a corpus plan's clips from their start presses and derive the curves.

## A new clip that takes a branch way

To add a clip that sends one branch a given way (`$D005` falls through, say), use gp's own commands, not a script of your own that replays from power-on. A probe recipe plays every try from one console made at its start, so a sweep tries hundreds of timings in minutes.

1. Read the branch: `gp code listing ROM --routine D005` prints its routine. Write the way as the RAM test that decides it, from the compare's operands. In Donkey Kong, `$D005` (`cpy #$10; bmi`) falls through when sub_CFC6 finds both points sprites' y (`$02C0`, `$02C8`) not 255, so the probe measure is a score change on a frame both are busy: `changes score_mid while pts_a != 255 and pts_b != 255`.
2. Write a probe in a scratch probes file (the format is in [measuring the original](../original-measures.md#gp-mechanic-probe); copy the `[watch]` names you need from the game's `probes.toml`). Its `base` is a `[start.NAME]` whose steps are the clip's start presses (`start@60-69` in a 599-frame `.presses` file is `". 60", "T 10", ". 529"`), a stored path, or a session's clip. A base can't name a corpus clip by its ID. Sweep what you don't know (`sweep = { W = [0, 1, 2] }`, `{W}` in the steps) and put the measure in `check`.
3. `gp mechanic probe GAME --file FILE --jobs 2` prints each cell's measures. Narrow in stages: a cheap sweep first (where Mario stands, and `first state == 255` for when a barrel reaches him), then the timing sweep only around that.
4. Keep the cell that holds: its steps with numbers in place of the sweep, `expect` on the measure, and a `session` named after no clip already in the corpus (`gp corpus import` names a clip `PREFIX.SESSION.NNN`, so a session called `third-points` would overwrite `dk.third-points.001`). `gp mechanic probe GAME --file FILE --save` writes it into the session under `$GP_HOME/mechanics/GAME`. A probe with no `session` saves nothing and says nothing.
5. `gp clip score ../games/GAME SESSION:N` scores it on the game's model and gives the first wrong frame.
6. `gp corpus import GAME` without `--write` must list only the new clip as added; then `--write` writes it into `corpus.toml` and its `inputs/ID.start.presses` and `inputs/ID.presses`.
7. Check the way: `gp code listing ROM --clips ../games/GAME/corpus.toml --only ID --routine D005` shows the branch's taken and fell counts on the clip.

`gp mechanic probe --import SESSION:N` prints a saved session clip as a probe to start from. `gp code listing` and `gp clip score` restore a clip's start from the start-snapshot cache; `gp mechanic probe`, `gp explore` and `gp reach` replay their starts from power-on once, which costs a second or so.

The other searches don't take a branch way or a RAM test as a goal. `gp explore ROM --start SET --from ID --no-starts --target sub_XXXX` searches from the end of a start clip toward a routine no start clip runs, and writes clips from power-on with `--write DIR`. `gp farm` plays a level with a decision server, scored by progress. `gp run-until REC 'COND'` reads a recording of the store, not a clip, and prints the first frame the condition holds.

In game-printer, remaking `dk.third-points.001` this way from its start (frame 599) took two sweeps: 15 standing places (4.5 s), then 30 jump frames before each barrel's arrival (450 cells, 4 min 51 s on two threads). 58 cells took the way, at x 145, 149, 152, 154 and 160; the hand clip's (x 154) was among them. The x 145 cell, saved, imported and scored, ran 1101 of 1101 frames together on the model, and `gp code listing` counted `$D005` fell once.
