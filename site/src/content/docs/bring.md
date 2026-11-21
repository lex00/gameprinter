---
title: Bring in a game
description: Port a game's mechanics from a ROM you own, step by step, for an NES game and an arcade game.
---

Porting a game starts with a ROM you own. You play it and record clips, and your agent reads the game's code and writes down how it plays: what moves, how fast, what happens on contact. That written version is the game's model, `model.toml`, and the clips prove it matches the original. Your part is mostly playing and saying what looks wrong.

The finished folder holds the model and nothing from the ROM. That makes it a template anyone can copy to start a game of their own. The ported game plays in a browser, reading its art and levels from each player's own ROM inside the page, and it never becomes a new ROM. Donkey Kong plays that way today. No arcade game plays in a browser yet.

So a port is not a remake of someone else's game. gp reads your ROM on your machine the way an emulator does, the model is the game's rules written as text and checked against clips of your ROM, no ROM bytes or art from it go in any repo or release, and the port plays only for someone who has the ROM. Ask your agent to port your ROM and it starts here.

## The steps, in order

This is the checklist from game-printer, the older tool (its `docs/new-game.md`), with the command that does each step today. Donkey Kong's first stage taught us the order (`docs/research/dk-25m-retrospective.md` in the gameprinter repo). Set up the corpus and the gate before the first clip, and every clip gets scored the same way from the start. Put the player's place in the state mapping before the first clip joins the corpus too: a clip scored over no fields agrees on every frame and tells you nothing.

In the table, GAME is the game's folder in your games checkout (`games/my-game`) and RUN is a scoring run's name. The page is the one `gp serve` hosts and `gp mechanic start` opens. Its tabs are CLIPS, REFINE, PLAY and THEMES. The sections after the table go into each part.

| # | Step | NES | Arcade (a MAME romset) | On the page | Done when |
|---|---|---|---|---|---|
| 1 | Copy the ROM in and identify it | `gp roms add ROM`, then `gp roms scan` | The same, with the set's zip | | `roms scan` names the file with its hash. A ROM never goes in a repo |
| 2 | Compare revisions, if there are several | `gp roms diff A.nes B.nes --summary` | `gp roms diff` on two sets' zips | | One revision is chosen as the target |
| 3 | Start the game | `gp game new GAME --rom ROM --genre G` | The same, `--genre other` | | The game's folder, its model skeleton, the corpus plan and the quest log are written |
| 4 | Check the skeleton | `gp model check GAME`, `gp template check GAME --rom ROM`, `gp package scan GAME` | The same | | All three pass |
| 5 | Land the skeleton | `gp model gate GAME --patch FILE -m MSG` in the games checkout | The same | | It is on the games branch: the gate takes a patch that adds a game the tip doesn't have |
| 6 | Verify the source | An edit to the game's `roms.toml`: `status = "verified"` with its evidence | The same; a set is known by its chips' SHA-1s from MAME's source | | It can wait; nothing downstream needs it |
| 7 | Find a movie that syncs | `gp movie import FILE.fm2` (or `.bk2`), `gp movie diagnose` | `gp movie import FILE.inp`, a MAME recording of the set | | A run of the first stage plays in the emulator, or none does (`docs/movies.md`) |
| 8 | Name the stage byte | An edit to `observe.toml`'s `[level]`, from RAM at each stage of the movie; then `gp mechanic harvest GAME RECORDING --add` cuts it into events per stage and adds them to the quest log | The `[level]` edit, from `gp query` on a recording; harvest runs on the board too | | The quest log has the stages |
| 9 | Find the player's bytes | Record a walk clip, then `gp model blind DIR --rom ROM`, or read the clip's RAM; `gp labels lookup` names what's there | Record a walk clip and read its RAM; `gp labels propose` drafts a labels file from a run's access log; `gp model blind DIR --rom SET.zip` finds the player on the board too | CLIPS: `gp mechanic start GAME walk`, R records, Enter hands it off | The player's x and y are `[[field]]`s in `mapping.toml`, each with the clip and frames it was read from |
| 10 | Set the scope | An edit to `mapping.toml`'s `[scope]` (the stub `gp game new` wrote) | The same | | Scoring stops where a run leaves the stage the model holds |
| 11 | Record the first clips | A row of the quest log at a time; `gp corpus import GAME --write`, or `gp model gate GAME --import` | The same; `gp clip presses GAME ID --from-session SESSION/N` keeps one clip as presses | CLIPS: click a row, play, record, hand off | The clips are in `corpus.toml` and `inputs/` |
| 12 | Find the random draws | `gp interp draws --find ROM --input LOG`, with LOG from `gp clip log GAME SESSION:N` | The same; it reads the main Z80's code | | Its proposed `[[draws]]` are in `mapping.toml` with `score_draws = true` (`gp game new` already tried an idle run) |
| 13 | Score a clip | `gp clip score GAME SESSION:N`, or `gp corpus score GAME --run RUN --only ID` | The same; `gp clip screen` checks the screen measure on the board first | CLIPS: a clip's ON THE MODEL | Its frames together, its pairing and its first split |
| 14 | Measure what a clip shows | `gp mechanic probe GAME --import SESSION:N`, then `gp mechanic probe GAME` | The same | | The measures are probes in `probes.toml`, so they run again with no agent |
| 15 | Model each mechanic | The element-extraction skill (`.claude/skills/element-extraction/SKILL.md`): `gp element inputs --run GAME --clip ID` for the routine behind a wrong byte; `gp model refine` for the numbers | The same; `element inputs` and `code listing` read the Z80 | REFINE: the background rounds, what each kept, its questions | Each row of the quest log is done from the gate's scores (`gp mechanic queue GAME`) |
| 16 | Land each change | `gp model gate GAME --patch FILE -m MSG` | The same | REFINE shows the gate's scores; PLAY plays the landed model with its settings | PASS: no clip at 100% got worse and no contract bar was lost |
| 17 | Ask for clips no sweep can make | `gp mechanic leads GAME --add`, `gp mechanic sweep jump GAME` (the jumpathon: a grid of jumps on both games) | `gp mechanic leads` runs on the board; the sweep is NES-only for now, so ask for jumps from the quest log | CLIPS: the rows the leads add | The person is asked only for what the tools can't make |
| 18 | Check the picture | `gp picture GAME --run RUN` | The same | CLIPS: ON THE MODEL's overlay and differences | The picture gap is measured from the first model, not after the behaviour is done |
| 19 | Record the held-out runs | The sealed-run row of the quest log | The same | CLIPS: Select or Tab in review seals a run | Full runs of the first stage are under `~/.gameprinter/sealed/GAME/`, never in a session's clips |
| 20 | Score the held-out runs | `gp sealed score GAME RUN`, the coordinator alone | The same | | Each run's line is in `~/.gameprinter/sealed/GAME/ledger.txt` |

Once the model holds, THEMES lists the game's themes and its theme kit; [Themes and levels](../themes/) says how to make one. A few things `gp game new` doesn't do yet. It doesn't find a movie that syncs (try each published movie of the ROM's hash with `gp movie diagnose`), and it doesn't find the player's or the stage's bytes. It also writes the region `ntsc` for every new NES entry, so correct that for a PAL dump.

## 1. Put your ROM in your ROM folder

ROMs live in one folder on your machine, `$GP_ROM_DIR` (`~/.gameprinter/roms` unless you set it), and never in a repo. Copy a ROM you own there:

```sh
gp roms add ~/Downloads/my-game.nes
```

Each file gets named after the game whose `roms.toml` in your [games checkout](https://github.com/lex00/games) holds its hash. A ROM that no game names yet is added as unknown, and that's the one you start a new game from. `--move` moves the file instead of copying it. `--clean` adds a copy without the junk some dumps carry (a trainer, bytes after the game data) and notes what it removed.

A ROM hack's entry in `roms.toml` (`kind = "patched"`) holds the hashes of the base ROM, the author's patch and the patched image. Game Printer doesn't apply patches. Make the patched image yourself from your own base, outside Game Printer, and `gp roms add` will know it by its hash like any other dump. The patch and the patched image never go in a repo.

`gp roms scan` lists what's in the folder and which game each file belongs to.

If the game has several revisions, compare them and pick one as the target. `gp roms diff a.nes b.nes --summary` lists each range of PRG and CHR that differs, named from the first ROM's asset map and labels (graphics, level data, a table, code), without quoting any bytes. When the two programs sit at different offsets, `--cpu C000-FFFF` compares what the CPU sees there instead. `docs/roms-diff.md` in the gameprinter repo has the rest.

Another dump of a game that already has a folder may keep the same assets at other offsets. `gp assets find GAME --rom ROM` looks for each of the game's assets by the content its asset map records, and says where each one is and whether it changed. When no game knows the ROM, it prints a draft `roms.toml` entry, and `--lift DIR` lifts what it found. `gp assets hash ROM` writes that content into the asset map from your own ROM, as hashes only. On an arcade zip, find compares the files with every chip of each set the game's map lists, by SHA-1, whatever the files are called. `docs/assets.md` has the rest.

## 2. Start the game's folder

```sh
gp game new my-game --rom ~/.gameprinter/roms/nes/my-game.nes --genre platformer --title "My Game"
```

This makes `my-game/` in your games checkout with `package.toml`, `roms.toml` (the ROM's hashes) and an asset map of what the ROM's header says. It also writes a model skeleton for the genre: a pad, a player and an empty level, each marked as a placeholder. The genres are `platformer`, `topdown`, `gridhop` (hopping on a grid of cells, as in Q*bert), `events` (track and field) and `other`. There's a corpus with no clips yet, and pins that already hold against your ROM. No byte of the ROM is written. It never writes over anything, and it refuses a ROM another game already names.

The ROM's source in `roms.toml` is named after the game. When the game will have more than one ROM, `--source ID` gives it another name (`qbert-u`). The six ROMs game-printer registered have entries in `roms/known.toml` in the gameprinter repo, and if yours is one of them, `roms.toml` gets its entry with the board facts and evidence.

Check what it made:

```sh
gp model check my-game
gp template check my-game --rom ~/.gameprinter/roms/nes/my-game.nes
gp package scan my-game
```

If you keep `gp package scan --json` output as a baseline (`--baseline FILE`), a later scan treats what it found then as known and fails only on something new.

The first real job is finding where the game keeps the player in RAM and adding that to `mapping.toml`, before the first clip.

## 3. Play, record and talk to the agent

`gp game new` also starts the game's quest log, a list of the mechanics to record for its genre (the title screen, walking, jumping, enemies and so on), each with a line saying what to show. In your gameprinter checkout, build the browser player once, then open the page on one of them:

```sh
scripts/web/build-play.sh
gp mechanic start my-game walk
```

It opens the page in `gp serve` at `http://127.0.0.1:7710/mechanic/my-game/?tab=clips`, on your machine only (it starts `gp serve` if none is running), and prints a line to give your agent. `--rom FILE` names the ROM instead of finding it in your ROM folder.

The page has four tabs:

- CLIPS is where you record and review. It shows the quest log, the game in the middle (the original, THE ORIGINAL, or a clip on the game's model, ON THE MODEL), and a line about the background work underneath.
- REFINE is that background work: the clips being scored and refined on the game's model, what each round kept, and its questions.
- PLAY is the game's model by itself, with its settings.
- THEMES has the game's themes: its theme kit, making and fitting a theme, the readability and originality checks, and each theme's clip review and refinement ([Themes](../themes/)).

AGENT'S LOG in the header (or L) shows what your agent is doing, beside any tab.

Play with the keyboard or a controller. R (or the right bumper) starts and stops a recording. Type a note and press Enter to hand the clip to your agent. The agent reads the session with `gp mechanic show my-game walk` and replies with `gp mechanic say`. Its words show over the game, which waits for your answer. Click another row of the quest log to work on that next. In PLAY, your agent's words wait until Esc takes you back to CLIPS.

`gp mechanic start --own-server` serves the page on a server of its own, as before. When a new `gp` is published to `~/.gameprinter/bin/gp` (by the gate, or `gp self publish`), that server restarts into it between requests, on the same port and mechanic; `--no-follow` keeps it on the build it started with.

Once a mechanic's clips are in the corpus and a patch lands through `gp model gate`, the gate writes its scores to `~/.gameprinter/work/refine/my-game/status.json`. The quest log (`gp mechanic queue my-game`, and the list on the page) then shows each row's scores: how many clips meet the contract's bars, the share of frames that agree, and the first split. When all of a row's clips meet the bars, the row shows as done.

If you worked on a game with game-printer, the old tool, `gp mechanic import my-game --from ~/.game-printer` brings over its mechanic sessions, quest log, scores and sealed runs, under the game's name here. The old name becomes the corpus's import prefix, such as donkey-kong-oe for donkey-kong. `--dry-run` lists what it would copy, and it never overwrites anything.

Clips are kept as presses, never pictures, under `~/.gameprinter/mechanics/my-game/`.

You can score a session's clips on the model before they're in the corpus. `gp clip score my-game walk:3` scores clip 3 of the walk session, `gp clip score my-game walk` scores all of them, and `--all` scores every clip of every session. `gp clip log my-game walk:3` writes the clip's input from power-on, checking its start against the state saved with it. A sealed session (one whose name starts with `sealed`) is refused, and `--all` leaves it out.

Next to the game, Your agent shows what the agent is up to while you play. You see its status and a log (`gp mechanic log`), pictures and clips it pins for you to look at (`gp mechanic pin`, such as a sheet of frames or the original against the model), and a table of the problems it has found and where each stands (`gp mechanic problem`). Tell it what you see in the conversation.

Each clip in the list has an ON THE MODEL button. It plays the clip through the original and through the game's model, the way the score does, and shows them side by side, the model over the original, or just their differences, frame by frame. A strip under the pictures is green where they agree and red where they differ, and the DIFF buttons jump between differences. A game with custom code (hooks, Rust compiled to WebAssembly) runs as its compiled module. Start the page with `--module FILE` (`gp game build` makes it), or keep the module at `~/.gameprinter/work/modules/GAME.wasm`.

## 4. Put the clips in the corpus

The clips you hand off stay in the session until they join the game's corpus, the set of clips its model is scored against:

```sh
gp corpus import my-game
gp corpus import my-game --write
```

The first line tells you which clips it would add, change and leave out, and the version step. With `--write` it puts them in `corpus.toml` and `inputs/` and records the new version. Clips already there stay as they are, and nothing is removed unless you name it with `--remove`. `--session walk` takes only that session's clips. Sealed runs, rows marked out of scope and anything `[import] exclude` names stay out. If you edit `corpus.toml` by hand, `gp corpus bump my-game` says what changed since the recorded version, and `--write` records the next one. The format and the version rules are in `docs/formats/scoring-corpus.md` in the gameprinter repo.

In a games repo the import can go through the gate instead. `gp model gate my-game --import` runs the import from your sessions on the branch's tip, gates the result as a patch (scoring the corpus before and after), and lands it if nothing got worse. If the sessions hold nothing new, it lands nothing. `--removes ID` lets it remove a clip, as with a patch.

A corpus grows a session at a time, and a lot of clips end up running the same code. `gp corpus reduce my-game` proposes the fewest clips that still run every routine and branch way the corpus runs in each stage. Clips that your fix commits name are always kept. Without `--write` nothing changes; `--write ~/.gameprinter/work/reduce.toml` writes the plan, with each clip kept or dropped and why. On an arcade romset it counts both Z80s' code. The rules are in `docs/formats/scoring-corpus.md`, "Reduce".

## 5. Measure what the original does

Once the mapping has the player and a few clips are in, your agent measures the original and holds the model to it. These commands read RAM and the state mapping, and work the same on an NES game and an arcade game:

- `gp mechanic probe my-game` runs the probe recipes in `my-game/probes.toml`: a start, some presses, and measures read from RAM after each frame, such as the frames from a jump's press to its landing. Each measure with an expected value is checked, so the facts a mechanic rests on run again with no agent.
- `gp interp moments` lists the game's own moments in a run (jumps, landings, deaths, lives lost, counters, level changes) and whether the model does each one.
- `gp interp translate` shows the original's state at a frame as a model state, part by part, and what the mapping can't carry over.
- `gp interp scenarios` runs short presses on both games from the same start, one button at a time, then in combinations.
- `gp interp slots` and `gp interp odds` hold the model's enemies to the original's enemy slots, and its random choices to the odds its own rules give.
- `gp picture my-game --run RUN` and `gp solve my-game --run RUN` go through a scored corpus: the first puts each picture difference down to a cause, the second proposes the model numbers that would close a clip's gap.

For a game with custom code (hooks), give these `--module FILE`, since the hooks run as a compiled module. [Measuring the original](https://github.com/lex00/gameprinter/blob/main/docs/original-measures.md) has each command in full.

## Tools for reading the game

Your agent reads the original with these, and you can too.

A game can keep a labels file, `labels.toml` in its folder, naming its RAM variables, routines and data tables. `gp labels show my-game` lists it, `gp labels lookup my-game '$0400'` (an address or a name) finds what's there, and `gp labels check my-game/mapping.toml` lists every RAM address the state mapping reads that has no label. The format is in `docs/formats/labels.md` in the gameprinter repo.

`gp model mechanics my-game --module ~/.gameprinter/work/modules/my-game.wasm` lists the mechanics the model uses, from the records its components and hooks declare, with the class of every model path and the hooks that are still game-specific code. It exits 1 when the model and a record disagree. A model with no hooks needs no `--module`. `docs/mechanics.md` in the gameprinter repo has the catalog's parts.

## An arcade game

An arcade game comes in as a romset, a MAME zip named after its MAME system, such as `rbtapper.zip`. Game Printer recognises a set by its chips and ignores the file names inside. Each chip's SHA-1 has to match the set's list from MAME's own source. The set's identity, the `sha256` in its `roms.toml` and `package.toml`, is the SHA-256 of its chips' SHA-1s, sorted, one per line. Any zip holding those chips has the same identity. So far the known sets are Tapper's: `rbtapper`, `tapper`, `tappera`, `tapperg` and `sutapper`.

```sh
gp roms add ~/Downloads/rbtapper.zip
gp roms scan
gp game new tapper --rom ~/.gameprinter/roms/arcade/rbtapper.zip --genre other --title Tapper
```

`gp roms add` checks the zip against the sets and copies it whole to `$GP_ROM_DIR/arcade/`. `gp game new` writes the game's folder much as for an NES game, with these differences:

- `platform = "arcade"`, and the MAME set in `system`
- a model on the arcade controls (coin, start1, a joystick and button1) at 30 frames a second, with a 512x480 screen
- a state mapping marked `platform = "arcade"`, whose `ram[...]` reads the main CPU's addresses (Tapper's work RAM is at `0xE000`)

Its asset map, `assets/tapper.toml`, describes the board: the 1024 background tiles (`gfx1`) and 256 sprites (`gfx2`) by index. Those are the same in every Tapper set, and the same model draws with whichever set's art a player has (`tapper`'s, or `tapperg`'s other sprites). `gp model check`, `gp template check` (the romset's identity, the map and what it lifts) and `gp package scan` all work on it.

Lift the pictures from whichever listed set your zip is into a folder outside any repo, and check that they pack back to the chips:

```sh
gp assets lift ~/.gameprinter/roms/arcade/tapper.zip --map ../games/tapper/assets/tapper.toml -o ~/.gameprinter/work/assets/tapper
gp assets pack ~/.gameprinter/work/assets/tapper --check ~/.gameprinter/roms/arcade/tapper.zip
```

Each picture is an indexed PNG of 4-bit values with gray previews, and `source.toml` records which set they came from. The colours aren't in the chips. The board looks them up in palette RAM, which the game writes as it runs, and a model has to do the same.

To play it, build the arcade player once, then open the page:

```sh
scripts/web/build-play.sh --arcade
gp mechanic start tapper walk
```

As in MAME, 5 drops a coin and 1 starts. The arrows move, and X or Space taps. On a controller, Select is the coin, Start starts and A taps. Recording, notes, hand-offs, talk and FREE PLAY work as for an NES game. Clips are kept in the MAME oracle's input log format (`gp-arcade-input 0`), and MAME can replay any clip as it is. ON THE MODEL has nothing to show on an arcade game yet: Tapper has a model, but no model draws an arcade board's pictures yet. The board's sound plays in stereo the way MAME makes it, the first AY-8910 on the left and the second on the right. U turns it off and on.

To keep a clip in the game's corpus, save it as presses:

```sh
gp clip presses ../games/tapper tapper.serve.001 --from-session walk/1 --name "Serving at three bars"
```

This writes `inputs/tapper.serve.001.start.presses` (power-on to the clip's start) and `inputs/tapper.serve.001.presses` (the clip), adds the clip to `corpus.toml` and remakes its `recipes` hash. An arcade game's files give the set and buttons on their third line, `arcade tapper CSRLDUT`, and call the buttons `coin`, `start1`, `up`, `down`, `left`, `right` and `tap` (`start1@120-125`, `up+tap@300-340`). The files are read back the way the scorer reads them, and the clip is replayed to the state its session recorded. `--log FILE --start N` takes a clip from any input log that starts at power-on instead, such as a MAME oracle log.

Before a model is held to the screen measure, check the measure on the original against itself:

```sh
gp clip screen ../games/tapper tapper.score.001 --mask ~/.gameprinter/work/tapper-mask.png
```

It plays the clip on the board live, keeping a score tape, then plays it again from that tape, and measures each frame's two pictures with the sprite mask (every pixel a sprite drew). Every frame has to come out exact. `--mask` writes a frame beside its mask in red. Scoring an arcade clip keeps the board's side in a score tape, as for the NES, keyed by the romset's identity, so a long clip doesn't replay the board every time.

Tapper's board has habits a model must copy to match it frame for frame. Model each one as a quirk switch in the profiles, as the NES games do, rather than as fixed behaviour. Retro is the original's behaviour, and Normal turns the quirk off:

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

Also still missing: games on a cartridge type the emulator doesn't handle yet. Those need a [change to the tool](../contribute/) first.

[Improving a game](../improve/) uses the same model, clips and gate, if you'd like to start there.

## Recordings, the event stream and press searches

These commands read the original's code as it runs, an NES ROM on gp-nes and an arcade romset on gp-arcade.

`gp record --source my-game-rom --input run.input` keeps a run of the original in the store. `gp replay`, `gp query`, `gp run-until`, `gp trace` and `gp events` read it back at any frame, down to its RAM, its CPU, every instruction, its interrupts and its bank switches. `gp movie import` turns a published FCEUX or BizHawk movie into an input log for it. An arcade romset records the same way from an arcade input log, and `gp movie import` reads a MAME `.inp` recording of it. `docs/recording.md` in the gameprinter repo has the rest.

`gp watch start my-game ~/roms/my-game.nes` runs stage 0 on a ROM, with no model. It reads the code, runs seeded starts, and searches for presses that reach code no run has reached. Each new thing it finds (new code, banks, jumps, pointer values) goes to `~/.gameprinter/watch/my-game/events.ndjson` as it goes. Your agent follows along with `gp watch events my-game --follow`, and a page can follow the `/api/watch/my-game/stream` route of the mechanic page's server. `gp explore` is the search on its own: it writes the presses it keeps as a clip set. `gp capture dense games/my-game` searches from your game's clip starts for a few short clips that each run much more of the original's code. `docs/watch.md` and `docs/capture.md` have the rest.

## Reading the original's code

These read the NES's 6502 code, or the Z80's on an arcade romset. Where a romset can't do something yet, it says so below.

`gp model static ~/roms/my-game.nes --unread` reads everything the code settles without running it: its routines and dispatch, constants, state machines, the RAM a model will need, and where the reader stopped. `gp code listing ~/roms/my-game.nes --clips clips.toml` lists each routine with how often each instruction ran and each branch went each way over your clips, and `gp code tables ~/roms/my-game.nes --index '$53'` finds the tables a RAM byte (a stage counter) indexes. `gp reach` says what your clips ran, by routine with `--window`, and compares two windows or clip sets with `--diff-*`.

On an arcade romset, `gp model static ~/roms/arcade/tapper.zip` reads the main Z80's code alone, without running anything. It finds the interrupt handlers the CTC's vectors name, the routines and what each does (a frame wait, a jump table, the random routine), the record tables that loops walk with IX, IY or HL, and the state machines, counters, timers, input bytes and calls of the random routine. `--write`, `--model` and `--grade` work as for the NES; `--clips` and `--unread` don't yet.

`gp interp draws --find ~/roms/my-game.nes --input run.input` finds the game's random number generator and the places its code reads it, and proposes `[[draws]]` entries for your mapping; `gp game new` already runs it on a minute of no input. `gp element inputs --run games/my-game --clip CLIP` names the routine that wrote a wrong measure's byte on the clip's first wrong frame, and every byte and branch that decided it. `gp cycles` measures the CPU's work per frame, for a game that steps its random generator while it waits for the next frame. `docs/code-reading.md`, `docs/element.md`, `docs/formats/draws.md` and `docs/cycles.md` in the gameprinter repo have the rest.

## Stage 0 as data, and mechanics from a long run

These read the code and sprites of an NES ROM or of an arcade romset.

`gp dataset rom ~/roms/my-game.nes --out ~/.gameprinter/work/datasets/my-game` writes stage 0 of a ROM as tables: its routines and branches, the RAM it writes, its elements, state machines, constants, the mechanics it can name and what it leaves open. No file holds the ROM's bytes, and the command fails if one would. `gp dataset gaps DIR...` counts what datasets leave open. `docs/formats/rom-dataset.md` in the gameprinter repo has every table.

`gp dataset grade DIR` runs the ROM (or the arcade romset) from stage 0's starts and counts how much of what the run touched has a row in the dataset, and how much it leaves open. Given a game's key folder too, it grades the dataset against the key (`docs/formats/rom-dataset-key.md`); `--counts` prints counts only. No game in the games checkout has a key yet.

`gp mechanic harvest my-game RECORDING` replays a long recording once (a published speedrun, imported with `gp movie import`) and runs the mechanic sessions' detectors over all of it. The detectors look for head and side hits, landings and stomps, items and hurts, ignored input, changes nothing explains, and code and objects seen for the first time. Each kind gets an example clip per level in a `harvest-LEVEL` session, and `--add` puts the kinds in your quest log. `gp mechanic leads my-game` replays every session clip and lists the code and RAM that stage 0's runs reached and no clip has, by the routine it branches off. That's your list of what to play next.

Your agent can cite a clip as evidence. `gp mechanic contacts my-game walk 3` runs the detectors over one clip and writes, next to it, where the player touched something and what changed at that moment, for the page to draw. `gp mechanic compare my-game jump 4 2` answers a question the measurements settle as a turn in the session, such as the tallest jump of clip 4 against clip 2 (on an arcade game, how far across each clip took the player). `gp mechanic video my-game walk 3` records the clip after checking its start and draws it as an animated PNG under `~/.gameprinter/videos/`. Any recording can be drawn the same way with `gp video RECORDING`, NES or arcade, with every frame checked as it is drawn. A clip from an input log that runs from power-on is saved with `gp mechanic clip-save my-game walk --log FULL.log --start N`. `docs/video.md` has the rest.

## A first model from the ROM alone, and the jumpathon

`gp model blind ~/my-game-model --rom ~/roms/my-game.nes` plays the ROM with no model and no mapping. First it finds where play starts and which way is forward, the player's x and y bytes, and the jump and run buttons. Then it measures the walk and run speeds, the jump (takeoff speed, gravity and fastest fall) and the player's size. The measures become `observed` facts with the frames they came from, and the player's bytes become mapping fields. `--play-rounds N` adds a search over the first level with a decision model (`--decide`, as `gp farm` uses). It works on platformers whose start the blind discovery can reach; on Donkey Kong it stops at "play start: no start run reaches play". An arcade romset (`--rom tapper.zip`) plays on gp-arcade with the board's work RAM, its sprite list and the arcade device's buttons (a coin goes in before Start); the rest is the same rules. Where no button makes a jump, it looks for lanes: a stick way that moves the player's y by one amount a press and holds it there, with a lane number that changes on the same frame. A lane player gets its own measures: the button held until it acts (the count it runs, and what changes when it ends), the score (a decimal digit a byte, rising in play and never falling, its digits the ones the points routine writes), the search's moves built from those (walk, lane up and down, hold and let go) with the score as progress, the lane rule fitted from all of it with `gp rule lane`'s fitter and checked frame by frame, and the order of the frame's work from the board's access log. The mapping it writes (`entities.player[0].x`, `.y`, `.lane`, `counters.score`) is what `gp rule lane` reads; the output prints the command. On Tapper it finds the bartender's x, y and bar, the tap's pour and the score, with no model.

`gp mechanic sweep jump my-game` is the jumpathon. It finds open ground blind and makes a grid of jumps from it (takeoff speed, how long A is held, steering in the air), plays each on the original and on your model in lockstep, and shows which cells the model gets wrong first. `--along PATH` sweeps every place a path through a level stands on, and `--fit-blind DIR` fits the jump from the game's own clips into a scratch copy of the model. Both read the NES's RAM and sprite table, and refuse an arcade game in words for now.

## What the original shows that your model doesn't

These read a recording of the original (`gp record`) beside your game's folder, NES or arcade. On an arcade game `gp sprites capture` only proposes the model's sprites (leave out `--apply`), and `gp sound capture` reads the board's AY log.

`gp track RECORDING` follows the objects a recording draws with no mapping and finds the RAM bytes that hold them, and `--propose-slots` writes a `[slots]` section to start `mapping.toml` from. `gp gaps all games/my-game --recording RECORDING` lists what changes, reacts or makes a sound in the original that the model doesn't account for, catalogs the interactive things (a block that breaks, a pickup) and measures where the model draws each object against the original's sprites; the reports go to `~/.gameprinter/reports/gaps/my-game/`. `gp coverage RECORDING` says how much of the code, RAM, screens and buttons your recordings reached, and `gp coverage check` holds them to the thresholds in a coverage definition. `gp sprites capture games/my-game --recording RECORDING` collects every sprite frame of every entity into the lifted assets and proposes the model's sprites, and `gp sound capture --recording RECORDING --name level-1 --template games/my-game` turns the sound the original made into songs and effects; `gp sound fidelity` and `gp sound compare` hold them to the original's register log. `docs/gaps.md`, `docs/coverage.md`, `docs/capture-presentation.md` and `docs/sound.md` in the gameprinter repo have the rest.

`gp sound drivers` lists the sound engines the tools know, and `gp sound decode --source SOURCE` reads songs with a known driver's decoder. On Tapper the decoder reads the AY log a capture wrote (`--ay-log`) and takes each run of sound between silences as a song. `gp sound render SONGS --out FILE.wav` plays songs or a register log, Tapper's through its two AY-8910s. The SF2 and SFZ banks a mapping can send instruments to are listed by `gp sound banks`.

## What the tools found, as one record

`gp findings ingest my-game` collects what the tools have written about a game, one line per finding with its evidence and status, in `~/.gameprinter/findings/my-game.ndjson`. A finding stays a lead until a clip of real inputs shows it. `gp findings cut my-game` cuts those clips from a farm run's path (`gp farm --blind`) and the jumpathon's strips into the game's `findings` session. `gp findings confirm my-game ID` is the gate. It records the clip with `gp record`, checks it frame by frame with `gp replay`, and runs the detector for the finding's kind again. The finding is confirmed only if that detector fires, and at the finding's place when it has one. `--confirm` on `cut` runs the gate on every clip, and `gp findings list my-game` counts the findings by kind and status. On Tapper the same steps run on gp-arcade with the board's work RAM and sprite list.

## Holding the emulators to a reference

`gp oracle record ID` runs a recording's input in a reference emulator in its own Docker image. An NES recording runs in Mesen 2 (`just mesen-build` first) and is stored as a recording of its own; an arcade one runs in MAME (`just mame-build`) and is kept as a capture under `~/.gameprinter/work/oracle/`. `gp oracle diff A B` then compares the two frame by frame. For the NES the comparison is field by field; for an arcade game it holds gp-arcade's state and sound writes to the MAME capture. `gp oracle triage` and `golden` keep the NES verdicts in `oracle/triage.toml`, and `gp oracle check` (`just check-mesen`) holds the emulator to them. `gp testroms run` runs the standard NES test ROMs on gp-nes; `gp testroms fetch` downloads them at pinned hashes and `just build-testroms` builds our own. `docs/oracle.md` and `docs/test-roms.md` have the rest.

## Play no run has made yet

`gp localize my-game --level main` replays every clip and blind run of the game and finds the branches that only ever went one way in the level. For each one it traces back to the guard and puts it on the level grid (the mapping's `[cells]`), and a search goes there next unless you pass `--no-probe`. For now it refuses an arcade game and says why: it needs branch directions from gp-arcade's trace and a level grid in the mapping.
