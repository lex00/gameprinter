---
title: Improve a game
description: Change how a published game plays, check that nothing got worse, and send the change back.
prompt: |
  Read this page. Then help me find where the behaviour I describe next lives in the game's model.toml or its ext/ hook code, make the change in my fork of lex00/games, and run the gate with --dry-run. Show me its result before anything else.
---

Every game here is a model: text files that say how the game behaves. A set of recorded play clips, the corpus, shows whether the model still matches the original. You change the model, the gate plays every clip again, and the change is good if no clip got worse.

Today the game to improve is Donkey Kong and its open version, Rivet Rodeo. They share one model.

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

## Where things are

| In `donkey-kong/` | What it holds |
|---|---|
| `model.toml` | How the game behaves |
| `ext/` | Rust hook code, for what the model can't say |
| `mapping.toml` | Where the original keeps each value in memory |
| `fidelity.toml` | What has to match the original, and how closely |
| `corpus.toml`, `inputs/` | The clips: button presses only |
| `assets/` | Where the art is in a ROM: addresses, never the art |
| `open/` | Rivet Rodeo, the open version |

## Try something without a fork

`gp game fetch` puts a published version in a folder of its own. The folder has to be outside any git repo.

```sh
curl -s https://raw.githubusercontent.com/lex00/rivetrodeo/main/catalog.toml
gp game fetch lex00/donkey-kong@1.0.5 --id THE_ID_FROM_THE_CATALOG \
  --from https://raw.githubusercontent.com/lex00/rivetrodeo/main \
  --into ~/my-donkey-kong --rom-dir "$GP_ROM_DIR"
```

It checks every file against its hash and lifts the art from your ROM into the folder. That folder stays on your machine.

`gp model gate ~/my-donkey-kong -m "what you changed"` then gates your edits in place, keeping its history in a private repo of its own. To send the work back, make the same change in a fork as above.

## Not built yet

A way to send a change from a fetched folder directly, without a fork, and a credit for contributors in the published version.
