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

### Watch it work, and hand stuck clips to an agent

`gp serve` runs the same loop in the background while you play: its REFINE tab scores every clip on the model, refines the clip that parts earliest one round at a time, and shows each job as a card whose bars move as the round goes (the impact pass's trials, the fine pass's, and the fewest wrong frames so far). Put a clip first with "add to the queue" or "fix this clip".

A clip no number fixes can go to a clip agent: Claude, in a worktree of gameprinter of its own, posting each step and each attempt (scored on every clip, with a trial preview and a keep button) to the clip. It is off unless you turn it on: `GP_CLIP_AGENT=claude gp serve`. The commands it posts with (`gp agent note`, `gp agent attempt`, `gp agent done`) work by hand too; [docs/agents.md](https://github.com/lex00/gameprinter/blob/main/docs/agents.md) has the details.

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

## Not built yet

A way to send a change from a fetched folder directly, without a fork, and a credit for contributors in the published version.
