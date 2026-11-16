---
title: Improve a game
description: Fix something in how a published game plays, check nothing got worse, and send the fix back.
prompt: |
  Read this page. Then help me find where the behaviour I describe next lives in the game's model.toml or its ext/ hook code, make the change in my fork of lex00/games, and run the gate with --dry-run. Show me its result before anything else.
---

Spotted something Donkey Kong gets wrong? You can fix it. How a game plays lives in a text file of rules and numbers, `model.toml`, plus some Rust code (hooks) in games like Donkey Kong where the text isn't enough. Each game also keeps a set of recorded play clips, its corpus. After you change something, the gate plays every clip on the original and on your version, and your change is good if no clip got worse.

For now Donkey Kong is the game you can improve, played from a ROM you own.

## What you need

- [`gp` installed](../install/).
- Your own ROM of the game, in `GP_ROM_DIR`. The gate can't compare anything with the original without it. See the [ROM rules](../rom-rules/).

## Send a change

Clone your fork next to your `gameprinter` checkout, in the same parent folder.

```sh
gh repo fork lex00/games --clone
cd games
just setup
```

Make your change. Then save it as a patch and put the files back, and the gate tests the patch on a clean checkout.

```sh
git diff > /tmp/fix.patch
git checkout -- donkey-kong
gp model gate donkey-kong --patch /tmp/fix.patch --dry-run -m "Donkey Kong: what you changed"
```

The gate scores the corpus before and after the patch and says PASS or FAIL. It takes a few minutes, and with `--dry-run` it changes nothing.

If it passes, apply the patch again and open a pull request with the gate's result in the description. A maintainer runs the same gate and lands it. Never paste ROM data, lifted art or score output into the pull request.

A patch can only touch one game's folder. If it removes a clip on purpose, name the clip with `--removes`, or the gate refuses it.

## Let the clips find a number

When a number in `model.toml` is off (a jump's gravity, a walk's speed, a timer), `gp model refine` can work it out from the corpus. It plays each clip on your ROM and on the model side by side, starting from the original's state at the start of the clip, and tries new numbers for whichever mechanic goes wrong first. A value gets written only when that clip gets closer and no other clip gets worse.

It rewrites the model, so give it a copy outside any git checkout, plus the game's module built from its hook code:

```sh
mkdir -p ~/dk-refine
cp donkey-kong/model.toml donkey-kong/mapping.toml donkey-kong/fidelity.toml donkey-kong/profiles.toml ~/dk-refine/
gp game kit --out ~/gp-kit
gp game build donkey-kong --kit ~/gp-kit -o ~/dk.wasm
gp model refine donkey-kong --lockstep --model ~/dk-refine --module ~/dk.wasm
```

`--clip ID` (repeatable) limits it to those clips. `--against SCORE.json` takes a `gp game score` or `gp corpus score --json` output from before your change and first checks every clip against your ROM. A clip that no longer plays the same on the original is reported and left out.

Here's an example. Change Mario's jump gravity in a copy from `0.06640625` to `0.0625`, and 49 of Donkey Kong's 114 clips still play exactly. The first that doesn't is `dk.double-barrel-jump.003`, where Mario's height is wrong from frame 29 of the clip. Gravity moves that the most, and `0.06640625` makes the window exact. Every other clip is checked before the value is kept, and afterwards all 114 play exactly. On an unchanged copy it changes nothing.

For each clip it worked on, it prints the window it looked at, the number it changed, and the splits before and after. Each value it writes becomes a fact in `~/dk-refine/model.toml`, with the clip and frames as evidence. When no number fixes a window, it leaves the mechanic alone and asks you instead, in `~/dk-refine/questions.toml`. List and answer those with `gp model questions ~/dk-refine` (`--answer ID=N` picks an answer). Then copy the numbers you want into your fork and run the gate as above.

Numbers in a hook (an entity's `ext_params`, such as a walk's step or a hold's frames) get tried the same way. Change Kong's `hold` from 16 to 20 in a copy, and `gp model refine donkey-kong --lockstep --model ~/dk-refine --module ~/dk.wasm --clip dk.clear-25m.001` finds 16 again. `--mechanic ext:dk_throw` sticks to one hook's numbers, which is quicker. When no mechanic's numbers write a measure (a score the model only counts, say), it notes that the measure isn't held from that point and moves on to the next window a mechanic owns.

Without `--lockstep`, `gp model refine ~/dk-refine --recording ID` works from stored recordings (`gp record`) instead of the corpus. It measures the player's speeds and jump and the enemies' steps from the recording's sprites. Then it writes the values that a failing `gp validate` check backs up, asks you about the rest, and validates again. `gp model propose ~/dk-refine --recording ID` prints just the measures.

Two related commands work on their own. `gp validate ~/dk-refine --against ID --module ~/dk.wasm` holds the model to its contract (`fidelity.toml`). `gp model derive ~/dk-refine --assets LIFTED` reads each fact back from what its evidence cites (the ROM's lifted tables, a disassembly's lines) and asks you where they differ. `docs/validation.md`, `docs/derivation.md` and `docs/mechanics.md` in the gameprinter repo have the rest.

`gp mechanic start donkey-kong` serves the last run and the jump's rule (`gp rule jump donkey-kong`, the gravity and takeoff speeds read from the clips) at `http://127.0.0.1:PORT/refine`.

For a player who moves between lanes, the way Tapper's bartender moves between bars, there's `gp rule lane GAME`. It reads the player's lane, x and y bytes from the mapping (`entities.player[0].lane`, `.x`, `.y`, or `--lane`, `--x`, `--y`) and each lane's tap from `entities.tap[K].x` (or `--tap`). From the clips it fits:

- each lane's row
- how many frames a move to another lane takes, and how long it keeps the stick busy
- whether a move wraps round
- where the player lands (a set distance from the new lane's tap, or one x per lane)
- the walk's step per frame, and the step back after a send

Then it plays the rule against every clip, frame by frame, and reports for each clip the first frame where the rule and the original differ, and in what (lane, x or y). `gp rule lane tapper --lane 0xE6F9` fits Tapper's bartender. The result goes to `$GP_HOME/mechanics/GAME/lane/refine.json`, or to `--out`.

### Watch it work, and hand stuck clips to an agent

`gp serve` runs the same loop in the background while you play. Its REFINE tab scores every clip on the model and refines the clip that goes wrong earliest, one round at a time. Each job shows as a card whose bars move as the round runs: the impact pass's trials, the fine pass's, and the fewest wrong frames so far. To put a clip first, use "add to the queue" or "fix this clip". THE GRID in REFINE draws a small square for each of the model's mechanics in each level and context, coloured by how its clips score, and points to the empty square that most needs a clip.

A clip that no number fixes can go to a clip agent. That's Claude, working in its own worktree of gameprinter, posting each step and each attempt to the clip (scored on every clip, with a trial preview and a keep button). It's off until you turn it on with `GP_CLIP_AGENT=claude gp serve`. The commands it posts with (`gp agent note`, `gp agent attempt`, `gp agent done`) also work by hand. `gp agent brief --run GAME --clip ID` starts at a clip's first wrong frame, shows what the original's code read there (the Z80's, on an arcade game), and writes out the next commands. `gp agent tell` leaves the agent a note on a clip, and `gp agent clips` and `gp agent tells` list them. `gp agent integrate` merges kept attempts, and `gp agent bench` times the loop. [docs/agents.md](https://github.com/lex00/gameprinter/blob/main/docs/agents.md) has the details.

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

`gp game fetch` puts a published version in a folder of its own, which has to be outside any git repo.

```sh
gp game fetch lex00/donkey-kong@1.0.9 \
  --from https://raw.githubusercontent.com/lex00/gp-registry/main \
  --into ~/my-donkey-kong --rom-dir "$GP_ROM_DIR"
```

Without `--id`, it uses the id the registry's index lists for that version and prints it, saying where it came from. Without `@VERSION` too, it takes the newest version that isn't yanked (from a registry folder on your machine) or the catalog's latest (over https). A version yanked for security only comes with `--id`. It checks every file against its hash and lifts the art from your ROM into the folder, which stays on your machine.

Then `gp model gate ~/my-donkey-kong -m "what you changed"` gates your edits in place and keeps its history in a private repo of its own. To send the work back, make the same change in a fork as above.

## How far each game has come

`gp scorecard GAME...` prints a row per game with its asset map, slots, mechanics and hooks, its facts by source, and the numbers runs have recorded (fidelity, gaps, track, sound) ([the scorecard](https://github.com/lex00/gameprinter/blob/main/docs/scorecard.md)). `--module` reads the hooks from the game's module, and `--record` keeps every number as a metric.

Every `gp` command writes a run manifest under `~/.gameprinter/runs/` and a row in `~/.gameprinter/metrics.sqlite` ([run manifests](https://github.com/lex00/gameprinter/blob/main/docs/formats/run-manifest.md)). `gp runs list` shows the latest runs. `gp metrics record` and `gp metrics time -- CMD` add numbers, `gp metrics check --budgets FILE` holds them to limits, and `gp metrics report` draws their trends, with the scorecard, on one HTML page.

`gp corpus show DIR` prints a corpus's name, version, hash and changelog, and for a game its `[scored_against]` and score records. `gp corpus diff A B` lists the clips added, removed and changed between two corpora, and the version step that makes. Either one may be `REV:DIR`, the folder as a git revision has it. `gp corpus verify GAME --run RUN --module M` makes every clip again from the ROM through the game's module and checks it. A plan's curves are derived, so for a plan each clip is only made. `gp model diff A B` lists the facts that differ between two models, `gp model mutants DIR` writes the model's mutants (each number nudged, each flag flipped, each transition dropped), and `gp model schema WHICH` prints a format's JSON Schema. `gp input-log generate --frames N --hold start@120-124` writes a scripted input log (`--arcade tapper --columns CSRLDUT` for an arcade one), `gp input-log check FILE` counts its frames, `gp env` shows the paths gp uses, and `gp noop` records a run and nothing else. `gp bench baselines/gp-testrom` times a scripted baseline's recording and replay in a temporary store and keeps the numbers as metrics; an arcade baseline runs on gp-arcade (`docs/benchmarks.md`).

`gp progress --template GAME [--run GAME]` places the game on a ladder of nine stages, from the bare ROM to the whole game played identically. Each stage gets a number from 0 to 100 and a line saying what finishes it. `--measure` plays the game on the original and on the rebuilt game and keeps the result (add `--module` for a model with hooks). `--write-ignore MAPPING` adds the RAM bytes a trace says aren't game state to the mapping's ignore list (NES only). It works on an arcade romset too, through the game's own mapping on gp-arcade ([the progress ladder](https://github.com/lex00/gameprinter/blob/main/docs/progress.md)).

## Not built yet

A way to send a change from a fetched folder directly, without a fork, and a credit for contributors in the published version.
