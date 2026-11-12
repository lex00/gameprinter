---
title: Bring in a game
description: Start a new game from a ROM you own. The first steps work; the screen to play and record it is being brought over.
---

Bringing in a game starts from a ROM you own and ends with a model of the game and a set of clips that prove the model matches it. You'd normally direct an agent to do the reading and modelling, and step in to judge what you see when you play.

## 1. Put your ROM in your ROM folder

ROMs live in one folder on your machine, `$GP_ROM_DIR` (by default `~/.gameprinter/roms`), never in a repo. Copy a ROM you own there:

```sh
gp roms add ~/Downloads/my-game.nes
```

Each file is named by the game whose `roms.toml` in your [games checkout](https://github.com/lex00/games) holds its hash. A ROM no game names yet is added as unknown, and that's the one you start a game from. `--move` moves the file instead of copying it, and `--clean` adds a copy without the junk some dumps carry (a trainer, bytes after the game data), with a note of what was removed.

`gp roms scan` lists what's in the folder and which game each file belongs to.

If the game has several revisions, compare them and choose one as the target. `gp roms diff a.nes b.nes --summary` lists each range of PRG and CHR that differs, named by the first one's asset map and labels (graphics, level data, a table, code), without quoting any byte. When the two programs sit at different offsets, `--cpu C000-FFFF` compares what the CPU sees there instead. `docs/roms-diff.md` in the gameprinter repo has the rest.

## 2. Start the game's folder

```sh
gp game new my-game --rom ~/.gameprinter/roms/nes/my-game.nes --genre platformer --title "My Game"
```

This makes `my-game/` in your games checkout. It holds the package, the ROM's hashes in `roms.toml`, an asset map of what the ROM's header says, and a model skeleton for the genre (`platformer`, `topdown` or `other`): a pad, a player and an empty level, each marked as a placeholder. There's a corpus with no clips, and pins that already hold against your ROM. No byte of the ROM is written. It writes over nothing, and it refuses a ROM another game already names.

Check what it made:

```sh
gp model check my-game
gp template check my-game --rom ~/.gameprinter/roms/nes/my-game.nes
gp package scan my-game
```

The first real work is finding where the game keeps the player in RAM and adding it to `mapping.toml`, before the first clip.

## 3. Play, record and talk to the agent

`gp game new` also starts the game's quest log: the mechanics to record for its genre (the title screen, walking, jumping, enemies and so on), each with a line saying what to show. In your gameprinter checkout, build the browser player once, then open the page on one of them:

```sh
scripts/web/build-play.sh
gp mechanic start my-game walk
```

It serves the page at `http://127.0.0.1:7710/`, on your machine only, and prints a line to give your agent. Play with the keyboard or a controller. R (or the right bumper) starts a recording and stops it; type a note and press Enter to hand the clip to your agent. The agent reads the session with `gp mechanic show my-game walk` and answers with `gp mechanic say`; its words show over the game, which waits until you answer. Click another row of the quest log to work on it next. FREE PLAY is the game on its own: your agent's words wait there until Esc brings you back.

Clips are kept as presses, never pictures, under `~/.gameprinter/mechanics/my-game/`.

Beside the game, **Your agent** shows what the agent is doing while you play: its status and a log (`gp mechanic log`), pictures and clips it pins for you to look at (`gp mechanic pin`, such as a sheet of frames or the original against the model), and a table of the problems it has found with the state of each (`gp mechanic problem`). Tell it what you see in the conversation.

Each clip in the list has an **ON THE MODEL** button: it plays the clip through the original and through the game's model, the way the score does, and shows the two side by side, the model over the original, or their differences, frame by frame. A strip under the pictures is green where the two agree and red where they differ, and the DIFF buttons jump between differences. A game whose model has hooks runs in its module: start the page with `--module FILE` (`gp game build` makes it), or keep it at `~/.gameprinter/work/modules/GAME.wasm`.

## 4. Put the clips in the corpus

The clips you hand off stay in the session until they join the game's corpus, the clips its model is scored against:

```sh
gp corpus import my-game
gp corpus import my-game --write
```

The first says which clips it would add, change and leave out, and the version step; `--write` writes them into `corpus.toml` and `inputs/` and records the new version. A clip already there is kept as it is, and nothing is removed unless you name it with `--remove`. `--session walk` takes only that session's clips. Sealed runs, rows marked out of scope and anything `[import] exclude` names stay out. After editing `corpus.toml` by hand, `gp corpus bump my-game` says what changed since the recorded version and `--write` records the next one. The format and the version rules are in `docs/formats/scoring-corpus.md` in the gameprinter repo.

## Tools for reading the game

Your agent reads the original with these, and you can too.

A game can keep a labels file, `labels.toml` in its folder, naming its RAM variables, routines and data tables. `gp labels show my-game` lists it, `gp labels lookup my-game '$0400'` (an address or a name) finds what's there, and `gp labels check my-game/mapping.toml` lists every RAM address the state mapping reads that has no label. The format is in `docs/formats/labels.md` in the gameprinter repo.

`gp model mechanics my-game --module ~/.gameprinter/work/modules/my-game.wasm` lists the mechanics the model uses, from the records its components and hooks declare, with the class of every model path and the hooks that are still game-specific code. It exits 1 when the model and a record disagree. A model with no hooks needs no `--module`. `docs/mechanics.md` in the gameprinter repo has the catalog's parts.

## An arcade game

An arcade game comes in as a romset: a MAME zip named for its MAME system, such as `rbtapper.zip`. Game Printer knows a set by its chips, never by the file names inside: each chip's SHA-1 must match the set's list from MAME's own source. The set's identity, the `sha256` its `roms.toml` and `package.toml` give, is the SHA-256 of its chips' SHA-1s, sorted, one a line, so it is the same for any zip that holds those chips. The sets known so far are Tapper's: `rbtapper`, `tapper`, `tappera`, `tapperg` and `sutapper`.

```sh
gp roms add ~/Downloads/rbtapper.zip
gp roms scan
gp game new tapper --rom ~/.gameprinter/roms/arcade/rbtapper.zip --genre other --title Tapper
```

`gp roms add` checks the zip against the sets and copies it whole to `$GP_ROM_DIR/arcade/`. `gp game new` writes the package as for an NES game, with `platform = "arcade"` and the MAME set in `system`, a model on the arcade controls (coin, start1, a joystick and button1) at 30 frames a second with a 512x480 screen, and a state mapping marked `platform = "arcade"`, whose `ram[...]` reads the main CPU's addresses (Tapper's work RAM is at `0xE000`). There is no asset map yet: the tools don't lift arcade pictures. `gp model check`, `gp template check` (the romset's identity) and `gp package scan` work on it.

To play it, build the arcade player once, then open the page:

```sh
scripts/web/build-play.sh --arcade
gp mechanic start tapper walk
```

5 drops a coin and 1 starts, as in MAME; the arrows move and X or Space taps. On a controller, Select is the coin, Start starts and A taps. Recording, notes, hand-offs, talk and FREE PLAY work as for an NES game. Clips are kept in the MAME oracle's input log format (`gp-arcade-input 0`), so MAME can replay any clip as it is. **ON THE MODEL** says why it has nothing to show until the game has a model. The board makes no sound yet.

Also still missing: support for a game on a cartridge type the emulator doesn't handle yet, which would first need a [change to the tool](../contribute/).

In the meantime, [improving a game](../improve/) uses the same model, clips and gate.
