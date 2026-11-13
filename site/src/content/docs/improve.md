---
title: Improve a game
description: Change how a published game plays, check that nothing got worse, and send the change back.
prompt: |
  Read this page. Then help me find where the behaviour I describe next lives in the game's model.toml or its ext/ hook code, make the change in my fork of lex00/games, and run the gate with --dry-run. Show me its result before anything else.
---

Every game here is a model: text files that say how the game behaves. A set of recorded play clips, the corpus, shows whether the model still matches the original. You change the model, the gate plays every clip again, and the change is good if no clip got worse.

Today the game to improve is Donkey Kong, played from a ROM you own.

## What you need

- [`gp` installed](../install/).
- Your own copy of the game, in `GP_ROM_DIR`. The gate compares the model with the original, so it can't run without one. See the [ROM rules](../rom-rules/).

## Send a change

Clone your fork beside your `gameprinter` checkout, so the two folders share a parent.

```sh
gh repo fork lex00/games --clone
cd games
just setup
```

Make your change, then save it as a patch and put back the files, so the gate tests the patch on a clean checkout.

```sh
git diff > /tmp/fix.patch
git checkout -- donkey-kong
gp model gate donkey-kong --patch /tmp/fix.patch --dry-run -m "Donkey Kong: what you changed"
```

The gate scores the corpus before and after the patch and says PASS or FAIL. With `--dry-run` it changes nothing. It takes a few minutes.

On a PASS, apply the patch again and open a pull request with the gate's result in its description. A maintainer runs the same gate and lands it. Never paste ROM data, lifted art or score output into the pull request.

A patch can only change one game's folder. If it removes a clip on purpose, name the clip with `--removes`, or the gate refuses it.

## Let the clips find a number

When a number in `model.toml` is off (a jump's gravity, a walk's speed, a timer), `gp model refine` can find it from the corpus for you. It plays each clip on your ROM and on the model side by side, from the original's state at the clip's start, and tries the numbers of the mechanic that goes wrong first. A value is written only when that clip gets closer and no other clip gets worse.

It rewrites the model, so give it a copy outside any git checkout, and the game's module, built from its hook code:

```sh
mkdir -p ~/dk-refine
cp donkey-kong/model.toml donkey-kong/mapping.toml donkey-kong/fidelity.toml donkey-kong/profiles.toml ~/dk-refine/
gp game kit --out ~/gp-kit
gp game build donkey-kong --kit ~/gp-kit -o ~/dk.wasm
gp model refine donkey-kong --lockstep --model ~/dk-refine --module ~/dk.wasm
```

`--clip ID` (repeatable) works on those clips only. `--against SCORE.json`, a `gp game score` or `gp corpus score --json` output from before your change, first checks every clip against your ROM: a clip that no longer plays the same on the original is reported and left out.

An example: with Mario's jump gravity in a copy changed from `0.06640625` to `0.0625`, 49 of Donkey Kong's 114 clips play exactly. The first that doesn't is `dk.double-barrel-jump.003`, wrong from frame 29 of the clip in Mario's height. Gravity moves it the most, and `0.06640625` makes the window exact. Every other clip is checked before the value is kept, and all 114 play exactly after. On an unchanged copy it changes nothing.

What it prints, for each clip it worked on: the window it looked at, the number it changed and the splits before and after. Each value it writes is a fact in `~/dk-refine/model.toml` with the clip and frames as its evidence. When no number fixes a window, it doesn't change the mechanic: it asks you, in `~/dk-refine/questions.toml`. List and answer those with `gp model questions ~/dk-refine` (`--answer ID=N` picks an answer). Then copy the numbers you want into your fork and run the gate as above.

A number in a hook (an entity's `ext_params`, such as a walk's step or a hold's frames) is tried like any mechanic's number: with Kong's `hold` changed from 16 to 20 in a copy, `gp model refine donkey-kong --lockstep --model ~/dk-refine --module ~/dk.wasm --clip dk.clear-25m.001` finds 16 again. `--mechanic ext:dk_throw` keeps it to one hook's numbers, which is quicker. A measure that no mechanic's numbers write (a score the model only counts) is noted as not held from there, and the loop goes on to the next window a mechanic owns.

Without `--lockstep`, `gp model refine ~/dk-refine --recording ID` works from stored recordings (`gp record`) instead of the corpus: it measures the player's speeds and jump and the enemies' steps from the recording's sprites (`gp model propose ~/dk-refine --recording ID` prints those measures alone), writes the ones a failing `gp validate` check backs, asks about the rest, and validates again. `gp validate ~/dk-refine --against ID --module ~/dk.wasm` holds the model to its contract (`fidelity.toml`) on its own, and `gp model derive ~/dk-refine --assets LIFTED` reads each fact back from what its evidence cites (the ROM's lifted tables, a disassembly's lines) and asks where they differ. `docs/validation.md`, `docs/derivation.md` and `docs/mechanics.md` in the gameprinter repo have the rest.

`gp mechanic start donkey-kong` serves the last run and the jump's rule (`gp rule jump donkey-kong`, the gravity and takeoff speeds read from the clips) at `http://127.0.0.1:PORT/refine`.

A player who moves between lanes, as Tapper's bartender moves between the bars, has `gp rule lane GAME`. It reads the player's lane, x and y bytes from the mapping (`entities.player[0].lane`, `.x`, `.y`, or `--lane`, `--x`, `--y`) and each lane's tap from `entities.tap[K].x` (or `--tap`), and fits from the clips: each lane's row, the frames a move to another lane takes and keeps the stick busy, whether a move wraps round, where the player lands (so far from the new lane's tap, or one x for each lane), the walk's step a frame, and the step back after a send. It then plays the rule against every clip frame by frame and says, for each clip, the first frame where the rule and the original differ and in what (lane, x or y). `gp rule lane tapper --lane 0xE6F9` fits Tapper's bartender. The result goes to `$GP_HOME/mechanics/GAME/lane/refine.json`, or `--out`.

### Watch it work, and hand stuck clips to an agent

`gp serve` runs the same loop in the background while you play: its REFINE tab scores every clip on the model, refines the clip that parts earliest one round at a time, and shows each job as a card whose bars move as the round goes (the impact pass's trials, the fine pass's, and the fewest wrong frames so far). Put a clip first with "add to the queue" or "fix this clip". THE GRID in REFINE draws a small square for each of the model's mechanics in each level and context, colored by how its clips score, and names the empty square to record a clip for next.

A clip no number fixes can go to a clip agent: Claude, in a worktree of gameprinter of its own, posting each step and each attempt (scored on every clip, with a trial preview and a keep button) to the clip. It is off unless you turn it on: `GP_CLIP_AGENT=claude gp serve`. The commands it posts with (`gp agent note`, `gp agent attempt`, `gp agent done`) work by hand too. `gp agent brief --run GAME --clip ID` starts from a clip's first wrong frame and gives what the original's code read there (on an arcade game, the Z80's), with the next commands written out. `gp agent tell` leaves the agent a note on a clip; `gp agent clips` and `gp agent tells` list them, and `gp agent integrate` merges kept attempts (`gp agent bench` times the loop); [docs/agents.md](https://github.com/lex00/gameprinter/blob/main/docs/agents.md) has the details.

## Where things are

| In `donkey-kong/` | What it holds |
|---|---|
| `model.toml` | How the game behaves |
| `ext/` | Rust hook code, for what the model can't say |
| `mapping.toml` | Where the original keeps each value in memory |
| `fidelity.toml` | What has to match the original, and how closely |
| `corpus.toml`, `inputs/` | The clips: button presses only |
| `assets/` | Where the art is in a ROM: addresses, never the art |

## Try something without a fork

`gp game fetch` puts a published version in a folder of its own. The folder has to be outside any git repo.

```sh
curl -s https://raw.githubusercontent.com/lex00/gp-registry/main/catalog.toml
gp game fetch lex00/donkey-kong@1.0.9 --id THE_ID_FROM_THE_CATALOG \
  --from https://raw.githubusercontent.com/lex00/gp-registry/main \
  --into ~/my-donkey-kong --rom-dir "$GP_ROM_DIR"
```

It checks every file against its hash and lifts the art from your ROM into the folder. That folder stays on your machine.

`gp model gate ~/my-donkey-kong -m "what you changed"` then gates your edits in place, keeping its history in a private repo of its own. To send the work back, make the same change in a fork as above.

## How far each game has come

`gp scorecard GAME...` prints one row per game: its asset map, slots, mechanics and hooks, facts by source, and the numbers runs have recorded (fidelity, gaps, track, sound) ([the scorecard](https://github.com/lex00/gameprinter/blob/main/docs/scorecard.md)). `--module` reads the hooks from the game's module, and `--record` keeps every number as a metric. Every `gp` command writes a run manifest under `~/.gameprinter/runs/` and a row in `~/.gameprinter/metrics.sqlite`: `gp runs list` shows the latest, `gp metrics record` and `gp metrics time -- CMD` add numbers, `gp metrics check --budgets FILE` holds them to limits, and `gp metrics report` draws their trends, with the scorecard, as one HTML page ([run manifests](https://github.com/lex00/gameprinter/blob/main/docs/formats/run-manifest.md)).

`gp corpus show DIR` prints a corpus's name, version, hash and changelog, and for a game its `[scored_against]` and score records. `gp corpus diff A B` lists the clips added, removed and changed between two corpora and the version step that makes; either may be `REV:DIR`, the folder as a git revision has it. `gp corpus verify GAME --run RUN --module M` makes every clip again from the ROM through the game's module and checks it; a plan's curves are derived, so each clip is only made. `gp model diff A B` lists the facts that differ between two models, `gp model mutants DIR` writes the model's mutants (each number nudged, each flag flipped, each transition dropped), and `gp model schema WHICH` prints a format's JSON Schema. `gp input-log generate --frames N --hold start@120-124` writes a scripted input log (`--arcade tapper --columns CSRLDUT` for an arcade one), `gp input-log check FILE` counts its frames, `gp env` shows the paths gp uses, and `gp noop` records a run and nothing else. `gp bench baselines/gp-testrom` times a scripted baseline's recording and replay in a temporary store and keeps the numbers as metrics; an arcade baseline runs on gp-arcade (`docs/benchmarks.md`).

`gp progress --template GAME [--run GAME]` puts the game on a ladder of nine stages from the ROM to the whole game played identically. Each stage gets a number from 0 to 100 and a line saying when it is done. `--measure` plays the game on the original and on the rebuilt game and keeps the result (`--module` for a model with hooks); `--write-ignore MAPPING` writes the RAM bytes a trace says aren't game state into the mapping's ignore list (NES only). It works on an arcade romset too, through the game's own mapping on gp-arcade ([the progress ladder](https://github.com/lex00/gameprinter/blob/main/docs/progress.md)).

## Not built yet

A way to send a change from a fetched folder directly, without a fork, and a credit for contributors in the published version.
