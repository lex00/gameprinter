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

Not here yet: the tab that plays a clip on the original and the model side by side.

Also still missing: support for a game on a cartridge type the emulator doesn't handle yet, which would first need a [change to the tool](../contribute/).

In the meantime, [improving a game](../improve/) uses the same model, clips and gate.
