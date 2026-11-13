---
title: Bring in a game
description: Start a new game from a ROM you own, from copying the ROM in to a model scored on runs it has never seen, step by step, for an NES game and an arcade game.
---

Bringing in a game starts from a ROM you own and ends with a model of the game and a set of clips that prove the model matches it. You'd normally direct an agent to do the reading and modelling, and step in to judge what you see when you play.

## The steps, in order

This is game-printer's new-game checklist (its `docs/new-game.md`), with the command that does each step now. The order is the one Donkey Kong's first stage taught (`docs/research/dk-25m-retrospective.md` in the gameprinter repo). The corpus and the gate come before the first clip, so every clip is scored the same way from the start. The player's place goes in the state mapping before the first clip joins the corpus, since a clip scored over no fields agrees on every frame. GAME is the game's folder in your games checkout (`games/my-game`), RUN a scoring run's name. The page is `gp serve`'s, opened by `gp mechanic start`; its tabs are CLIPS, REFINE, PLAY and THEMES. The sections after the table say more about each part.

| # | Step | NES | Arcade (a MAME romset) | On the page | Done when |
|---|---|---|---|---|---|
| 1 | Copy the ROM in and identify it | `gp roms add ROM`, then `gp roms scan` | The same, with the set's zip | | `roms scan` names the file with its hash. A ROM never goes in a repo |
| 2 | Compare revisions, if there are several | `gp roms diff A.nes B.nes --summary` | `gp roms diff` on two sets' zips | | One revision is chosen as the target |
| 3 | Start the game | `gp game new GAME --rom ROM --genre G` | The same, `--genre other` | | The package, its model skeleton, the corpus plan and the quest log are written |
| 4 | Check the skeleton | `gp model check GAME`, `gp template check GAME --rom ROM`, `gp package scan GAME` | The same | | All three pass |
| 5 | Land the skeleton | `gp model gate GAME --patch FILE -m MSG` in the games checkout | The same | | It is on the games branch: the gate takes a patch that adds a game the tip doesn't have |
| 6 | Verify the source | An edit to the game's `roms.toml`: `status = "verified"` with its evidence | The same; a set is known by its chips' SHA-1s from MAME's source | | It can wait; nothing downstream needs it |
| 7 | Find a movie that syncs | `gp movie import FILE.fm2` (or `.bk2`), `gp movie diagnose` | `gp movie import FILE.inp`, a MAME recording of the set | | A run of the first stage plays in the emulator, or none does (`docs/movies.md`) |
| 8 | Name the stage byte | An edit to `observe.toml`'s `[level]`, from RAM at each stage of the movie; then `gp mechanic harvest GAME RECORDING --add` cuts it into events per stage and adds them to the quest log | The `[level]` edit, from `gp query` on a recording; harvest is NES-only for now | | The quest log has the stages |
| 9 | Find the player's bytes | Record a walk clip, then `gp model blind DIR --rom ROM`, or read the clip's RAM; `gp labels lookup` names what's there | Record a walk clip and read its RAM; `gp labels propose` drafts a labels file from a run's access log. `model blind` is NES-only | CLIPS: `gp mechanic start GAME walk`, R records, Enter hands it off | The player's x and y are `[[field]]`s in `mapping.toml`, each with the clip and frames it was read from |
| 10 | Set the scope | An edit to `mapping.toml`'s `[scope]` (the stub `gp game new` wrote) | The same | | Scoring stops where a run leaves the stage the model holds |
| 11 | Record the first clips | A row of the quest log at a time; `gp corpus import GAME --write`, or `gp model gate GAME --import` | The same; `gp clip presses GAME ID --from-session SESSION/N` keeps one clip as presses | CLIPS: click a row, play, record, hand off | The clips are in `corpus.toml` and `inputs/` |
| 12 | Find the random draws | `gp interp draws --find ROM --input LOG`, with LOG from `gp clip log GAME SESSION:N` | The same; it reads the main Z80's code | | Its proposed `[[draws]]` are in `mapping.toml` with `score_draws = true` (`gp game new` already tried an idle run) |
| 13 | Score a clip | `gp clip score GAME SESSION:N`, or `gp corpus score GAME --run RUN --only ID` | The same; `gp clip screen` checks the screen measure on the board first | CLIPS: a clip's ON THE MODEL | Its frames together, its pairing and its first split |
| 14 | Measure what a clip shows | `gp mechanic probe GAME --import SESSION:N`, then `gp mechanic probe GAME` | The same | | The measures are probes in `probes.toml`, so they run again with no agent |
| 15 | Model each mechanic | The element-extraction skill (`.claude/skills/element-extraction/SKILL.md`): `gp element inputs --run GAME --clip ID` for the routine behind a wrong byte; `gp model refine` for the numbers | The same; `element inputs` and `code listing` read the Z80 | REFINE: the background rounds, what each kept, its questions | Each row of the quest log is done from the gate's scores (`gp mechanic queue GAME`) |
| 16 | Land each change | `gp model gate GAME --patch FILE -m MSG` | The same | REFINE shows the gate's scores; PLAY plays the landed rebuild with its settings | PASS: no clip at 100% got worse and no contract bar was lost |
| 17 | Ask for clips no sweep can make | `gp mechanic leads GAME --add`, `gp mechanic sweep jump GAME` (the jumpathon: a grid of jumps on both games) | Both are NES-only for now; ask from the quest log | CLIPS: the rows the leads add | The person is asked only for what the tools can't make |
| 18 | Check the picture | `gp picture GAME --run RUN` | The same | CLIPS: ON THE MODEL's overlay and differences | The picture gap is measured from the first model, not after the behaviour is done |
| 19 | Record the held-out runs | The sealed-run row of the quest log | The same | CLIPS: Select or Tab in review seals a run | Full runs of the first stage are under `~/.gameprinter/sealed/GAME/`, never in a session's clips |
| 20 | Score the held-out runs | `gp sealed score GAME RUN`, the coordinator alone | The same | | Each run's line is in `~/.gameprinter/sealed/GAME/ledger.txt` |

Once the model holds, THEMES lists the game's themes and its theme kit: [Themes and levels](../themes/) has how to make one. `gp game new` doesn't yet find a movie that syncs (try each published movie of the ROM's hash with `gp movie diagnose`) or the player's and the stage's bytes, and it writes the region `ntsc` for a new NES entry, which you correct for a PAL dump.

## 1. Put your ROM in your ROM folder

ROMs live in one folder on your machine, `$GP_ROM_DIR` (by default `~/.gameprinter/roms`), never in a repo. Copy a ROM you own there:

```sh
gp roms add ~/Downloads/my-game.nes
```

Each file is named by the game whose `roms.toml` in your [games checkout](https://github.com/lex00/games) holds its hash. A ROM no game names yet is added as unknown, and that's the one you start a game from. `--move` moves the file instead of copying it, and `--clean` adds a copy without the junk some dumps carry (a trainer, bytes after the game data), with a note of what was removed.

A ROM hack is a patched source: a game's `roms.toml` names the base it is made from and the author's patch by its hash (`kind = "patched"`). Download the patch from its author's page yourself, then `gp roms add --patch hack.ips` applies it to your own base in the ROM folder, checks every hash the entry gives, and writes the result to `nes/patched/` with a note of how it was made. `--base ID` picks one base, and `--new --base ID` applies a patch nobody has registered yet and prints a draft entry, writing nothing. The patch and the result never go in a repo.

`gp roms scan` lists what's in the folder and which game each file belongs to.

If the game has several revisions, compare them and choose one as the target. `gp roms diff a.nes b.nes --summary` lists each range of PRG and CHR that differs, named by the first one's asset map and labels (graphics, level data, a table, code), without quoting any byte. When the two programs sit at different offsets, `--cpu C000-FFFF` compares what the CPU sees there instead. `docs/roms-diff.md` in the gameprinter repo has the rest.

A dump of a game that already has a folder may keep the same assets at other offsets. `gp assets find GAME --rom ROM` searches it for each of the game's assets by the content its asset map holds and says where each asset is and whether it changed. When no game knows the ROM it prints a draft `roms.toml` entry; `--lift DIR` lifts what it found. `gp assets hash ROM` writes that content into the asset map from your own ROM, hashes only. On an arcade zip, find compares the files with every chip of each set the game's map lists by SHA-1, whatever their names. `docs/assets.md` has the rest.

## 2. Start the game's folder

```sh
gp game new my-game --rom ~/.gameprinter/roms/nes/my-game.nes --genre platformer --title "My Game"
```

This makes `my-game/` in your games checkout. It holds the package, the ROM's hashes in `roms.toml`, an asset map of what the ROM's header says, and a model skeleton for the genre (`platformer`, `topdown`, `gridhop` for a hop on a grid of cells such as Q*bert, `events` for track and field, or `other`): a pad, a player and an empty level, each marked as a placeholder. There's a corpus with no clips, and pins that already hold against your ROM. No byte of the ROM is written. It writes over nothing, and it refuses a ROM another game already names.

The ROM's source in `roms.toml` is named after the game; `--source ID` names it otherwise (`qbert-u`), when the game will have more than one ROM. The six ROMs game-printer registered have their entries in `roms/known.toml` in the gameprinter repo, and `roms.toml` gets the one for yours with its board facts and evidence.

Check what it made:

```sh
gp model check my-game
gp template check my-game --rom ~/.gameprinter/roms/nes/my-game.nes
gp package scan my-game
```

`gp package scan --json` output kept as a baseline (`--baseline FILE`) lists what the scan found then as known, so a later scan fails only on something new.

The first real work is finding where the game keeps the player in RAM and adding it to `mapping.toml`, before the first clip.

## 3. Play, record and talk to the agent

`gp game new` also starts the game's quest log: the mechanics to record for its genre (the title screen, walking, jumping, enemies and so on), each with a line saying what to show. In your gameprinter checkout, build the browser player once, then open the page on one of them:

```sh
scripts/web/build-play.sh
gp mechanic start my-game walk
```

It opens the page in `gp serve`, at `http://127.0.0.1:7710/mechanic/my-game/?tab=clips` (on your machine only; it starts `gp serve` when none is running), and prints a line to give your agent. The page has four tabs. **CLIPS** is where you record and review: the quest log, the game in the middle on the original (THE ORIGINAL) or a clip on the game's model (ON THE MODEL), and under it a line of the background work. **REFINE** is the background work: the clips being scored and refined on the rebuilt game, what each round kept, and its questions. **PLAY** is the rebuild on its own, with its settings. **THEMES** is the game's themes: its theme kit, making and fitting a theme, the readability and originality checks, and each theme's clip review and refinement ([Themes](../themes/)). **AGENT'S LOG** in the header (or L) opens what your agent is doing beside any tab. `gp mechanic start --own-server` serves the page on a server of its own, as before; when a new `gp` is published to `~/.gameprinter/bin/gp` (by the gate, or `gp self publish`), that server restarts into it between requests, on the same port and mechanic, and `--no-follow` keeps it on the build it started with. `--rom FILE` names the ROM instead of finding it in your ROM folder. Play with the keyboard or a controller. R (or the right bumper) starts a recording and stops it; type a note and press Enter to hand the clip to your agent. The agent reads the session with `gp mechanic show my-game walk` and answers with `gp mechanic say`; its words show over the game, which waits until you answer. Click another row of the quest log to work on it next. In PLAY your agent's words wait until Esc brings you back to CLIPS.

Once a mechanic's clips are in the corpus and a patch lands through `gp model gate`, the gate writes its scores to `~/.gameprinter/work/refine/my-game/status.json`, and the quest log (`gp mechanic queue my-game`, and the page's list) shows each row's scores: how many clips meet the contract's bars, the share of frames together, and the first split. A row whose clips all meet the bars shows as done.

If you worked on a game with game-printer, the old tool, `gp mechanic import my-game --from ~/.game-printer` brings its mechanic sessions, quest log, scores and sealed runs into this home under the game's name here (the old name is the corpus's import prefix, such as donkey-kong-oe for donkey-kong). `--dry-run` lists what it would copy. It overwrites nothing.

Clips are kept as presses, never pictures, under `~/.gameprinter/mechanics/my-game/`.

A clip of a session is scored on the model before it's in the corpus: `gp clip score my-game walk:3` scores clip 3 of the walk session, `gp clip score my-game walk` every clip of it, and `--all` every clip of every session. `gp clip log my-game walk:3` writes the clip's input from power-on, its start checked against the state saved with it. A sealed session (its name starts with `sealed`) is refused, and left out of `--all`.

Beside the game, **Your agent** shows what the agent is doing while you play: its status and a log (`gp mechanic log`), pictures and clips it pins for you to look at (`gp mechanic pin`, such as a sheet of frames or the original against the model), and a table of the problems it has found with the state of each (`gp mechanic problem`). Tell it what you see in the conversation.

Each clip in the list has an **ON THE MODEL** button: it plays the clip through the original and through the game's model, the way the score does, and shows the two side by side, the model over the original, or their differences, frame by frame. A strip under the pictures is green where the two agree and red where they differ, and the DIFF buttons jump between differences. A game whose model has hooks runs in its module: start the page with `--module FILE` (`gp game build` makes it), or keep it at `~/.gameprinter/work/modules/GAME.wasm`.

## 4. Put the clips in the corpus

The clips you hand off stay in the session until they join the game's corpus, the clips its model is scored against:

```sh
gp corpus import my-game
gp corpus import my-game --write
```

The first says which clips it would add, change and leave out, and the version step; `--write` writes them into `corpus.toml` and `inputs/` and records the new version. A clip already there is kept as it is, and nothing is removed unless you name it with `--remove`. `--session walk` takes only that session's clips. Sealed runs, rows marked out of scope and anything `[import] exclude` names stay out. After editing `corpus.toml` by hand, `gp corpus bump my-game` says what changed since the recorded version and `--write` records the next one. The format and the version rules are in `docs/formats/scoring-corpus.md` in the gameprinter repo.

In a games repo the import can go through the gate instead: `gp model gate my-game --import` runs the import on the branch's tip from your sessions, gates the result as a patch (the corpus scored before and after) and lands it when nothing got worse. With nothing new in the sessions it lands nothing. `--removes ID` lets it remove a clip, as for a patch.

A corpus grows one session at a time, and many clips end up running the same code. `gp corpus reduce my-game` proposes the fewest clips that still run every routine and branch way the corpus runs in each stage. Clips your fix commits name are always kept. Without `--write` nothing changes; `--write ~/.gameprinter/work/reduce.toml` writes the plan with each clip kept or dropped and why. On an arcade romset both Z80s' code is counted. The rules are in `docs/formats/scoring-corpus.md`, "Reduce".

## 5. Measure what the original does

Once the mapping holds the player and a few clips are in, your agent measures the original and holds the model to it. These read RAM and the state mapping, so they work on an NES game and an arcade game alike:

- `gp mechanic probe my-game` runs the probe recipes in `my-game/probes.toml`: a start, some presses, and measures read from RAM after each frame, such as the frames from a jump's press to its landing. Each measure with an expected value is checked, so the facts a mechanic rests on run again with no agent.
- `gp interp moments` lists the game's own moments in a run (jumps, landings, deaths, lives lost, counters, level changes) and whether the model does each one.
- `gp interp translate` shows the original's state at a frame as a model state, part by part, and what the mapping can't carry over.
- `gp interp scenarios` runs short presses on both games from the same start, one button at a time, then in combinations.
- `gp interp slots` and `gp interp odds` hold the model's enemies to the original's enemy slots, and its random choices to the odds its own rules give.
- `gp picture my-game --run RUN` and `gp solve my-game --run RUN` go through a scored corpus: the first puts each picture difference down to a cause, the second proposes the model numbers that would close a clip's gap.

A game whose model has hooks runs them in its module, so give these `--module FILE`. [Measuring the original](https://github.com/lex00/gameprinter/blob/main/docs/original-measures.md) has each command in full.

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

`gp roms add` checks the zip against the sets and copies it whole to `$GP_ROM_DIR/arcade/`. `gp game new` writes the package as for an NES game, with `platform = "arcade"` and the MAME set in `system`, a model on the arcade controls (coin, start1, a joystick and button1) at 30 frames a second with a 512x480 screen, and a state mapping marked `platform = "arcade"`, whose `ram[...]` reads the main CPU's addresses (Tapper's work RAM is at `0xE000`). Its asset map, `assets/tapper.toml`, is the board's: the 1024 background tiles (`gfx1`) and 256 sprites (`gfx2`) by index, the same in every Tapper set, so the same model draws with whichever set's art a player has (`tapper`'s, or `tapperg`'s other sprites). `gp model check`, `gp template check` (the romset's identity, the map and what it lifts) and `gp package scan` work on it.

Lift the pictures, from whichever listed set the zip is, into a folder outside any repo, and check they pack back to the chips:

```sh
gp assets lift ~/.gameprinter/roms/arcade/tapper.zip --map ../games/tapper/assets/tapper.toml -o ~/.gameprinter/work/assets/tapper
gp assets pack ~/.gameprinter/work/assets/tapper --check ~/.gameprinter/roms/arcade/tapper.zip
```

Each picture is an indexed PNG of 4-bit values with gray previews, and `source.toml` says which set they came from. The colours aren't in the chips: the board looks them up in its palette RAM, which the game writes as it runs, so a model does the same.

To play it, build the arcade player once, then open the page:

```sh
scripts/web/build-play.sh --arcade
gp mechanic start tapper walk
```

5 drops a coin and 1 starts, as in MAME; the arrows move and X or Space taps. On a controller, Select is the coin, Start starts and A taps. Recording, notes, hand-offs, talk and FREE PLAY work as for an NES game. Clips are kept in the MAME oracle's input log format (`gp-arcade-input 0`), so MAME can replay any clip as it is. **ON THE MODEL** says why it has nothing to show until the game has a model. The board's sound plays in stereo, as MAME makes it: the first AY-8910 on the left, the second on the right. U turns it off and on.

To keep a clip in the game's corpus, save it as presses:

```sh
gp clip presses ../games/tapper tapper.serve.001 --from-session walk/1 --name "Serving at three bars"
```

This writes `inputs/tapper.serve.001.start.presses` (power-on to the clip's start) and `inputs/tapper.serve.001.presses` (the clip), adds the clip to `corpus.toml` and makes its `recipes` hash again. An arcade game's files say which set and buttons they're for on their third line, `arcade tapper CSRLDUT`, and name the buttons `coin`, `start1`, `up`, `down`, `left`, `right` and `tap` (`start1@120-125`, `up+tap@300-340`). The files are read back as the scorer reads them, and the clip is replayed to the state its session recorded. `--log FILE --start N` takes a clip from any input log from power-on instead, such as a MAME oracle log.

Before a model is held to the screen measure, check the measure on the original against itself:

```sh
gp clip screen ../games/tapper tapper.score.001 --mask ~/.gameprinter/work/tapper-mask.png
```

It plays the clip on the board live, keeping a score tape, then from that tape, and measures each frame's two pictures with the sprite mask (every pixel a sprite drew); every frame must come out exact. `--mask` writes a frame beside its mask in red. Scoring an arcade clip keeps the board's side in a score tape as for the NES, keyed by the romset's identity, so a long clip doesn't replay the board each time.

Tapper's board has ways a model of it must copy to match it frame for frame. Model each one as a quirk switch in the profiles, as for the NES games (Retro is the original's behaviour, Normal turns the quirk off), not as fixed behaviour:

| Quirk | What the board does |
|---|---|
| 30 Hz frame | One video frame is 1/30 s, so the model runs at 30 frames a second |
| Boot frames | The game reads no input for its first 94 frames while it tests its RAM |
| Doubled tiles | Background tiles are 8 by 8 in the chips and drawn 16 by 16; sprites move in steps of 2 pixels |
| Sprite pixel 8 | A sprite pixel of value 8 draws nothing but hides the sprites under it, so the tile shows there |
| No sprite limit | The sprite board draws all 128 sprites every frame: no flicker, no limit a line |
| Palette's ninth bit | Each colour is the byte of its pair the game wrote last, with a ninth bit saying which byte that was, so the palette depends on the order of writes, not only on what palette RAM holds |
| 4-way joystick | The joystick moves four ways: opposing directions cancel, and a diagonal keeps the direction that just changed (the horizontal one from rest). MAME filters a player's joystick so, and the mechanic page does too; a clip keeps what reached the board |
| Watchdog | The board resets itself after 16 frames without a write to the watchdog. A tilt during play hangs the game until it does |
| Sound: DC offset | MAME's AY table puts silence at -0.125, so each voice sits at an offset that changes with its volume |
| Sound: duty volume | The SSIO sets each voice's loudness by gating it with a duty cycle from its PROM, in 16 coarse steps |
| Sound: volume steps | The SSIO's volumes and mute act a 20 ms piece at a time (MAME's stream updates), not at the write |
| Sound: stereo by chip | Each AY has its own speaker |

Also still missing: support for a game on a cartridge type the emulator doesn't handle yet, which would first need a [change to the tool](../contribute/).

In the meantime, [improving a game](../improve/) uses the same model, clips and gate.

## Recordings, the event stream and press searches

These read the original's code as it runs. They are NES-only for now: an arcade game is refused in words.

`gp record --source my-game-rom --input run.input` keeps a run of the original in the store, and `gp replay`, `gp query`, `gp run-until`, `gp trace` and `gp events` read it back at any frame: its RAM, its CPU, every instruction, its interrupts and bank switches. `gp movie import` turns a published FCEUX or BizHawk movie into an input log for it. An arcade romset records the same way from an arcade input log, and `gp movie import` reads a MAME `.inp` recording of it. `docs/recording.md` in the gameprinter repo has the rest.

`gp watch start my-game ~/roms/my-game.nes` runs stage 0 on a ROM, with no model: it reads the code, runs seeded starts, searches for presses that reach code no run has, and writes each thing it finds first (new code, banks, jumps, pointer values) to `~/.gameprinter/watch/my-game/events.ndjson` as it goes. Your agent follows it with `gp watch events my-game --follow`, and a page with the `/api/watch/my-game/stream` route of the mechanic page's server. `gp explore` is the search on its own: it writes the presses it keeps as a clip set. `gp capture dense games/my-game` searches from your game's clip starts for a few short clips that each run much more of the original's code. `docs/watch.md` and `docs/capture.md` have the rest.

## Reading the original's code

These read the NES's 6502 code too, and refuse an arcade game in words for now.

`gp model static ~/roms/my-game.nes --unread` reads everything the code settles without running it: its routines and dispatch, constants, state machines, the RAM a model will need, and where the reader stopped. `gp code listing ~/roms/my-game.nes --clips clips.toml` lists each routine with how often each instruction ran and each branch went each way over your clips, and `gp code tables ~/roms/my-game.nes --index '$53'` finds the tables a RAM byte (a stage counter) indexes. `gp reach` says what your clips ran, by routine with `--window`, and compares two windows or clip sets with `--diff-*`.

On an arcade romset, `gp model static ~/roms/arcade/tapper.zip` reads the main Z80's code alone, with nothing run: the interrupt handlers the CTC's vectors name, the routines and what each does (a frame wait, a jump table, the random routine), the record tables loops walk with IX, IY or HL, state machines, counters, timers, input bytes, and each call of the random routine. `--write`, `--model` and `--grade` work as for the NES; `--clips` and `--unread` don't yet.

`gp interp draws --find ~/roms/my-game.nes --input run.input` finds the game's random number generator and the places its code reads it, and proposes `[[draws]]` entries for your mapping; `gp game new` already runs it on a minute of no input. `gp element inputs --run games/my-game --clip CLIP` names the routine that wrote a wrong measure's byte on the clip's first wrong frame, and every byte and branch that decided it. `gp cycles` measures the CPU's work per frame, for a game that steps its random generator while it waits for the next frame. `docs/code-reading.md`, `docs/element.md`, `docs/formats/draws.md` and `docs/cycles.md` in the gameprinter repo have the rest.

## Stage 0 as data, and mechanics from a long run

These read the NES's code and sprites too, and refuse an arcade game in words for now.

`gp dataset rom ~/roms/my-game.nes --out ~/.gameprinter/work/datasets/my-game` writes stage 0 of a ROM as tables: its routines and branches, the RAM it writes, its elements, state machines, constants, the mechanics it can name and what it leaves open. No file holds the ROM's bytes, and the command fails if one would. `gp dataset gaps DIR...` counts what datasets leave open. `docs/formats/rom-dataset.md` in the gameprinter repo has every table.

`gp dataset grade DIR` runs the ROM (or the arcade romset) from stage 0's starts and counts what the run touched that the dataset gives a row, and what it leaves open. Given a game's key folder too, it grades the dataset against the key (`docs/formats/rom-dataset-key.md`); `--counts` prints counts only. No game in the games checkout has a key yet.

`gp dataset export --run R --game games/my-game --out DIR` makes every clip of the corpus again into run R and scores it on the model. The tables are Parquet (or `--format csv`) with a dataset card and Croissant metadata. With `--hf DIR --template games/my-game` the export writes the game's data model instead, its facts with their evidence among it, as a Hugging Face style dataset repo. `gp dataset catalogue --out DIR games/a games/b` puts several games' facts and mechanics together with a taxonomy of mechanic parts. Both write only to the folder you give and check every file for runs of the ROM's bytes; nothing is published. A commercial game's per-frame measures stay out of `--hf` unless `--measures` is given (`docs/research/datasets.md`).

`gp mechanic harvest my-game RECORDING` replays a long recording (a published speedrun, imported with `gp movie import`) once and runs the mechanic sessions' detectors over all of it: head hits, side hits, landings, stomps, items, hurts, input ignored, changes nothing explains, code and objects seen for the first time. Each kind gets an example clip per level in a `harvest-LEVEL` session, and `--add` puts the kinds in your quest log. `gp mechanic leads my-game` replays every session clip and lists the code and RAM stage 0's runs reached that no clip has, by the routine it branches off: what to play next.

A clip is evidence your agent can cite. `gp mechanic contacts my-game walk 3` runs the detectors over one clip and writes where the player touched something, and what changed then, beside it for the page to draw. `gp mechanic compare my-game jump 4 2` answers a question the measurements settle as a turn in the session, such as the tallest jump of clip 4 against clip 2 (on an arcade game, how far across each clip took the player). `gp mechanic video my-game walk 3` records the clip after checking its start and draws it as an animated PNG under `~/.gameprinter/videos/`. Any recording can be drawn the same way with `gp video RECORDING`, NES or arcade, with every frame checked as it is drawn. A clip from an input log that runs from power-on is saved with `gp mechanic clip-save my-game walk --log FULL.log --start N`. `docs/video.md` has the rest.

## A first model from the ROM alone, and the jumpathon

`gp model blind ~/my-game-model --rom ~/roms/my-game.nes` plays the ROM with no model and no mapping: it finds play start, the player's x and y bytes, the jump and run buttons and the forward direction, then measures the walk and run speeds, the jump's takeoff speed, gravity and fastest fall, and the player's size, and writes them as `observed` facts with the frames they came from, and the player's bytes as mapping fields. `--play-rounds N` adds a search over the first level with a decision model (`--decide`, as `gp farm` uses). It works on platformers whose start the blind discovery can reach; on Donkey Kong it stops at "play start: no start run reaches play". An arcade romset (`--rom tapper.zip`) plays on gp-arcade with the board's work RAM, its sprite list and the arcade device's buttons (a coin goes in before Start); the rest is the same rules. Where no button makes a jump, it looks for lanes: a stick way that moves the player's y by one amount a press and holds it there, with a lane number that changes on the same frame. A lane player gets its own measures: the button held until it acts (the count it runs, and what changes when it ends), the score (a decimal digit a byte, rising in play and never falling, its digits the ones the points routine writes), the search's moves built from those (walk, lane up and down, hold and let go) with the score as progress, the lane rule fitted from all of it with `gp rule lane`'s fitter and checked frame by frame, and the order of the frame's work from the board's access log. The mapping it writes (`entities.player[0].x`, `.y`, `.lane`, `counters.score`) is what `gp rule lane` reads; the output prints the command. On Tapper it finds the bartender's x, y and bar, the tap's pour and the score, with no model.

`gp mechanic sweep jump my-game` is the jumpathon: a grid of jumps (takeoff speed, how long A is held, steering in the air) from open ground found blind, each played on the original and on your model in lockstep, and the grid says which cells the model gets wrong first. `--along PATH` sweeps every place a path through a level stands on, and `--fit-blind DIR` fits the jump from the game's own clips into a scratch copy of the model. Both read the NES's RAM and sprite table, and refuse an arcade game in words for now.

## What the original shows that your model doesn't

These read a recording of the original (`gp record`) beside your game's folder. They are NES-only for now: an arcade game is refused in words.

`gp track RECORDING` follows the objects a recording draws with no mapping and finds the RAM bytes that hold them, and `--propose-slots` writes a `[slots]` section to start `mapping.toml` from. `gp gaps all games/my-game --recording RECORDING` lists what changes, reacts or makes a sound in the original that the model doesn't account for, catalogs the interactive things (a block that breaks, a pickup) and measures where the model draws each object against the original's sprites; the reports go to `~/.gameprinter/reports/gaps/my-game/`. `gp coverage RECORDING` says how much of the code, RAM, screens and buttons your recordings reached, and `gp coverage check` holds them to the thresholds in a coverage definition. `gp sprites capture games/my-game --recording RECORDING` collects every sprite frame of every entity into the lifted assets and proposes the model's sprites, and `gp sound capture --recording RECORDING --name level-1 --template games/my-game` turns the sound the original made into songs and effects; `gp sound fidelity` and `gp sound compare` hold them to the original's register log. `docs/gaps.md`, `docs/coverage.md`, `docs/capture-presentation.md` and `docs/sound.md` in the gameprinter repo have the rest.

`gp sound drivers` lists the sound engines the tools know, and `gp sound decode --source SOURCE` reads songs with a known driver's decoder. On Tapper the decoder reads the AY log a capture wrote (`--ay-log`) and takes each run of sound between silences as a song. `gp sound render SONGS --out FILE.wav` plays songs or a register log, Tapper's through its two AY-8910s. Songs go out to FamiStudio or a MIDI file with `gp sound export` and come back with `gp sound import` (on Tapper, MIDI with `--ay`). The SF2 and SFZ banks a mapping can send instruments to are listed by `gp sound banks`.

## What the tools found, as one record

`gp findings ingest my-game` reads what the tools have written for a game and keeps one line per finding in `~/.gameprinter/findings/my-game.ndjson`, with its evidence and status. A finding is a lead until a clip of real inputs shows it. `gp findings cut my-game` cuts those clips from a farm run's path (`gp farm --blind`) and the jumpathon's strips into the game's `findings` session. `gp findings confirm my-game ID` is the gate. It records the clip with `gp record` and checks it frame by frame with `gp replay`, then runs the detector the finding's kind names again. Only if that detector fires, at the finding's place when it has one, is the finding confirmed. `--confirm` on `cut` runs the gate on every clip, and `gp findings list my-game` counts the findings by kind and status. On Tapper the same steps run on gp-arcade with the board's work RAM and sprite list.

## Holding the emulators to a reference

`gp oracle record ID` runs a recording's input in a reference emulator in its own Docker image. An NES recording runs in Mesen 2 (`just mesen-build` first) and is stored as a recording of its own; an arcade one runs in MAME (`just mame-build`) and is kept as a capture under `~/.gameprinter/work/oracle/`. `gp oracle diff A B` then compares the two frame by frame. For the NES the comparison is field by field; for an arcade game it holds gp-arcade's state and sound writes to the MAME capture. `gp oracle triage` and `golden` keep the NES verdicts in `oracle/triage.toml`, and `gp oracle check` (`just check-mesen`) holds the emulator to them. `gp testroms run` runs the standard NES test ROMs on gp-nes; `gp testroms fetch` downloads them at pinned hashes and `just build-testroms` builds our own. `docs/oracle.md` and `docs/test-roms.md` have the rest.

## Play no run has made yet

`gp localize my-game --level main` replays every clip and blind run of the game and finds the branches that only ever went one way in the level. For each it slices back to the guard and places that on the level grid (the mapping's `[cells]`). A search goes there next unless `--no-probe` is given. For now an arcade game is refused in words, since localize needs branch directions from gp-arcade's trace and a level grid in the mapping.
