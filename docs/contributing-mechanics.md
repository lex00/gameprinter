# Contributing a mechanic

This is how you add or fix one mechanic of one game: Mario's jump in Donkey Kong, the way a mug slides down the bar in Tapper. You play the game in the browser, record the moment the mechanic happens, and say in a sentence what it is. Your agent reads what the game did in that moment and comes back with what it learned and, sometimes, a question or a request for another clip. You keep going until the mechanic is settled, then send the change to the games repo.

Brought from game-printer's docs/contributing-mechanics.md. The loop is the same; the page is now a tab of `gp serve` ([mechanic-page.md](mechanic-page.md) has its layout, states and keys, which this page no longer repeats), games are package folders in the games checkout, and sealed runs have their own page ([sealed.md](sealed.md)).

One mechanic at a time. A few clips, each one understood, beat many clips nobody looked at.

The user, 2026-11-01: "this is how someone in the community would help. They would see their mechanic is missing from a game, or poorly implemented. then they would jump into the same loop ... to work with their own agent and tokens to identify all the refinements needed and process them."

## Status

Each step says what exists in gameprinter and what doesn't yet.

| Step | Exists | Not here yet |
|---|---|---|
| Setup and your ROM | `just setup`, `gp roms add`, `gp roms scan` | |
| Seeing what a game is missing | `gp mechanic queue GAME`, `gp mechanic leads GAME`, `gp dataset rom`, `gp dataset gaps` | `gp mechanic status GAME`: the game's mechanics, each known, estimated or missing |
| Starting in one command | `gp mechanic start GAME [MECHANIC] [--prompt TEXT]`: the session folder, the page on this machine, and the line to give your agent | A save point near the mechanic to start from |
| Playing and recording | Free play, F5 and F8 save points, and the controller's left and right triggers for save and load; clips as recipes from a save point | A record button of its own on the controller (R or the right bumper records) |
| Saying what a clip shows | A note on each clip, handed off with the controller's A or Enter; the agent's words over the paused game, with choices picked with the controller (`gp mechanic say GAME MECHANIC TEXT --choice ... --show CLIP`) | |
| The agent's answer | `gp mechanic show` (the session and whose turn it is), `gp mechanic wait`, `gp mechanic say`; `gp mechanic probe`; `gp element inputs` for the routine behind a wrong byte | `gp mechanic read CLIP`: the rules over one clip, giving what is now known and what is estimated |
| Checking and sending | `gp clip score GAME SESSION`, `gp mechanic probe GAME`, `gp corpus import GAME`, `gp model gate GAME`, the pre-commit hook, `gp dataset grade` | `gp mechanic check` |

## 1. Setup

```sh
git clone https://github.com/lex00/gameprinter && cd gameprinter
git clone https://github.com/lex00/games ../games
just setup
gp roms add "/path/to/your/donkey-kong.nes"
```

`gp roms add` copies your ROM into `$GP_ROM_DIR` (by default `~/.gameprinter/roms`) and names it by the game whose `roms.toml` in the games checkout holds its hash. A romset (a MAME zip, such as Tapper's) goes in the same way. Your ROM never goes into a repository; the hook refuses it.

## 2. Find the mechanic

```sh
gp mechanic queue donkey-kong
gp mechanic leads donkey-kong
```

The quest log lists the game's mechanics, each with its status (done, asking you, agent working, in line, next) and a line saying what is known or what it waits on. `leads` replays every clip and lists routines and RAM bytes stage 0 reached that no clip did, grouped by where they branch off: mechanics nobody has recorded yet. Pick a row that isn't done, or one you know is wrong because you've played the game.

## 3. Start

```sh
gp mechanic start donkey-kong jump
```

This opens the game's page in your browser, on `gp serve` at `http://127.0.0.1:7710/mechanic/donkey-kong/` (started when none is running), and starts a session your agent follows. It prints one line to give your agent; the `gp mcp` tools are in [mcp.md](mcp.md). For a game with several sets in its roms.toml, `--rom` picks one (Donkey Kong's OE or JU, Tapper's five).

The server follows the newest published build, so nobody rebuilds and restarts it by hand. It watches `$GP_HOME/bin/gp`, which the gate (or `gp self publish`) points at each build it publishes ([gate.md](gate.md)). When the link points at a new file, the server waits until no request is running, checks that the new file runs, and replaces itself with the new build on the same port, with the mechanic open in the page. `--no-follow` keeps the server on the build it started with.

## 4. The loop

| You | Your agent |
|---|---|
| Play to where the mechanic happens. Left trigger (F5) saves, right trigger (F8) loads, so you can try a moment again without touching the screen | Waits (`gp mechanic wait`) |
| Press R or the right bumper, do the mechanic, press it again. Write one sentence about it: "a short tap of A, then a full hold of A" | |
| Hand off with A or Enter; the panel shows the clip being sent, then "Clip N handed off" | Reads the clip: which bytes moved, which code wrote them, which constants it used. Answers over the game: what is now known ("the jump starts at a speed of -4, stored by the instruction at D95D"), what is estimated with its spread, and at most one question |
| Answer, or record the clip it asks for ("the same jump while running") | Can also make a clip or two itself from your save point, with presses it chooses (`gp mechanic probe --save`, `gp mechanic demo`), and says so |
| Repeat until the agent says the mechanic is settled, or you decide it's enough for now | Writes the mechanic's rows: known facts, estimates and what would settle them (`gp mechanic queue --known`, `--open`, `--note`) |

Keep each clip short (a few seconds) and about one thing. If the agent is unsure what a clip shows, it asks rather than guessing.

A clip belongs to a quest-log row, the open one when recording starts; clicking another row while recording or reviewing moves the clip there. The row is fixed when A or Enter is pressed and sent with the clip, so the server files it under that row whatever is open by the time it arrives. A clip handed off twice (the same play, start and inputs) is kept once.

A run that spans a stage (the stage `observe.toml`'s `[level]` reads changes during it, as a clear does) is offered for sealing when recording stops: a held-out run no worker sees. Select or Tab in review seals any clip. [sealed.md](sealed.md) says where sealed runs are kept, the guard that keeps every other tool off them, and how the coordinator scores them.

Any clip can be watched on the model: its ON THE MODEL button, or C (the left bumper) for the open row's newest clip. The page plays it through the original and the model the way the score does, with the original's random draws, and shows them side by side, the model over the original, or their differences ([mechanic-page.md](mechanic-page.md)). An arcade game has no model yet, so this says so in words.

### The quest log

The list of mechanics beside the game is the quest log (`$GP_HOME/mechanics/GAME/queue.json`, `gp mechanic queue GAME`). Its statuses and notes are typed by the agent, except where the gate has scored the row: a row whose clips are in the corpus takes its status and a line of scores from the gate's status file (`$GP_HOME/work/refine/GAME/status.json`, [gate.md](gate.md)), shown above the typed note.

Donkey Kong's log on 2026-11-02, in game-printer, from the gate's status after c610add3 (two rows):

```text
hammer: done. 4 of 4 clips meet the bars; 100.0% straight through; 100.0% with resync; longest exact stretch 12.1 s; 2 clips not in the corpus yet (gate, model c610add3)
    hammer: measured (819 frames, smash 500, no jump or climb), not yet in the rebuild: clips score 0 to 8%, clip 4 91% resync; worker A
clear-25m: agent working. 0 of 1 clips meet the bars; 89.3% straight through; 94.9% with resync; longest exact stretch 27.3 s; first split: clip 1 at frame 3447 (the clip's 1915), entities.player[0].x apart (gate, model c610add3)
    clear-25m: the 25m part scores 92.3% resync, 48.3% straight through; longest exact stretch 11.4 s; waits on the hammer and barrel drift
```

The hammer's typed note still quotes 0 to 8%; the line above it is the gate's.

| From the status file | What |
|---|---|
| Status | `done` when every clip of the row meets the contract's bars (100% of frames together where it sets none). Otherwise the typed status, and a row typed `done` shows `next` |
| Scores | The clips that meet the bars, the row's frames together straight through and with resync, its longest exact stretch, the first split of the first clip below the bars (the original's frame, the clip's frame and what differs), and how many of the session's clips aren't in the corpus yet |
| Note | Typed, unchanged: what a person reads. Scores are never typed |

A row the corpus shouldn't hold (a blind farm, title demos, another stage) is marked with `gp mechanic queue GAME ROW --out-of-scope WHY`, and `--in-scope` takes the mark off. `gp corpus import` leaves its clips out ([formats/scoring-corpus.md](formats/scoring-corpus.md)), and its line stays as typed.

### New clips join the corpus

A clip handed off stays in its session until it joins the game's corpus, the clips its model is scored against:

```sh
gp corpus import donkey-kong            # what it would add, change and leave out, and the version step
gp corpus import donkey-kong --write    # writes corpus.toml and inputs/, and records the version
gp model gate donkey-kong --import      # or: the import on the games repo's tip, gated and landed
```

Sealed runs never join it. Before it joins, a session's clip is scored with `gp clip score donkey-kong jump:3` (or `jump` for every clip of the session).

Not here yet: game-printer's `[gate] import = true` in a template's `loop.toml`, which had the page's server run the gate's import after each hand-off. loop.toml is gone; run the import yourself, or let the agent.

## 5. Check and send

```sh
gp mechanic probe donkey-kong
gp clip score donkey-kong jump
gp model gate donkey-kong --patch my.patch --dry-run
```

The probes run the mechanic's measures again from their recipes, the clip scores say how the model does on the session's clips, and the gate holds the change to the corpus: PASS or FAIL, and with no `--dry-run` it lands what passes. A change to a game goes to the games repo:

| In the change | Not in it |
|---|---|
| The clips as recipes (a save point's presses from power-on, and the presses), with your notes | The ROM, frames, sprites, RAM dumps or video |
| The mechanic's rows: known facts, estimates, open questions | |
| The model change, when the mechanic is a new option or a fixed number | |

Anyone with the same ROM makes the same clips again from the recipes and gets the same rows.

Not here yet: `gp mechanic check GAME MECHANIC`, which was to replay every clip of the session from its recipe and check that the rows still hold in one command.

### Probe recipes

A clip the agent makes (a probe) and the numbers it reads off are kept as a probe recipe in the game's `probes.toml`, so they run again with no agent:

```sh
gp mechanic probe donkey-kong                       # every probe: each measure checked
gp mechanic probe donkey-kong m50-pie-jump          # one probe
gp mechanic probe donkey-kong --import jump:3       # a saved clip, printed as a probe to start from
```

| Part | What it says |
|---|---|
| `base` | Where it starts: a named start from power-on, a stored path, or a session's clip, at a frame; `replay` plays the base's own inputs on |
| `steps` | Presses (`R 20`, `RA 1`), a walk to a RAM value (`walk_to x 28`), presses held until a RAM test holds (`until state == 2 U max 30`), the base's own inputs until a test holds (`replay_until floor == 3 and state == 1`), `repeat`, and `mark` (measures start there) |
| `sweep`, `rows` | A cell for each combination of values, and for each row of a `byte-rows` asset lifted from the ROM (the ladders of a stage) |
| `check` | Measures read from RAM (`first state == 255`, `values x at changes y`, `intervals y while state == 2`), each with the value it must have |
| `clips` | The session's clips it reproduces: compared input for input from power-on |

`--save` writes each cell with no clip yet into its session, as a clip in the snapshot format. The tool knows no game: the RAM names are the file's `[watch]` table.

## What the agent does, and what it doesn't

The agent reads the game's memory and code during your clip with the same rules stage 0 runs, and with `gp element inputs`, `gp code listing` and the game's labels for the routine behind a byte. It doesn't refine on its own: an estimate stays an estimate with its spread and what would settle it, for the scoring and refinement that runs in the background (the page's REFINE tab). It works with your agent and your tokens; nothing is sent anywhere but your agent.
