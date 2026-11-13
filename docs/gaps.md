# Gaps

Brought from game-printer's docs/gaps.md (port wave 4). In gameprinter a template is a game's package folder in the games checkout (`games/donkey-kong`) or a folder of `templates/` (`templates/mosshop`); `--dir` names where to look, by default `templates/` then `$GP_GAMES_DIR`. No metrics are recorded. The detectors read the NES's sprite table, nametables, APU and 6502 code, and an arcade recording's facts come from gp-arcade (wave 6b, the last section).

`gp gaps` finds what a recording of the original game shows that the model doesn't account for, and catalogs it. The question behind it comes from playing the regenerated game and finding things missing: which parts of the original change, react or make a sound with nothing in the model behind them. The command detects and catalogs only. Nothing it does changes a model; a later step turns its findings into model kinds and facts. The work belongs to Milestone 2, part of step 7 ([the roadmap](../ROADMAP.md)).

There are three detectors, each a subcommand, and `gp gaps all` runs all three from one read of the recording:

```sh
gp gaps changes donkey-kong --recording 53e7d93d      # unexplained changes, in five places
gp gaps interactive donkey-kong --recording 53e7d93d  # interactive elements, as catalog records
gp gaps drawn donkey-kong --recording 53e7d93d        # the model's sprite boxes against the original's
gp gaps all mosshop --recording ab5050be --listing target/mosshop/mosshop.dbg
```

| Option | Meaning |
|---|---|
| `--recording` | The recording of the original, by ID prefix. It needs a frame trace; its event stream is read when it has one |
| `--frames` | Frames to read, as for `gp track` (default all) |
| `--focus` | A window to explain (repeat for more): what the detectors found in those frames, with the routines and RAM rare elsewhere, written as `focus.json` |
| `--labels` | The game's labels file ([labels](formats/labels.md)), instead of `labels.toml` in the template's folder, this repo's `games/<source>/labels.toml` (Moss Hop's), or the one beside the games checkout's roms.toml that lists the source |
| `--listing` | An ld65 debug file. Its constants name sound effects; its code labels name routines for a board with 32KB of PRG or less when the labels file has none |
| `--dir` | The folder holding the templates (default `templates/`, then the games checkout) |
| `--truth` | A labelled reference set to score against, instead of the template's in `crates/gp-gaps/truth/` |
| `--oam-lag` | Frames the sprite table lags RAM, as for `gp track` (default 1) |
| `--assets` | The model's lifted assets, for `drawn` (default `$GP_HOME/work/assets/<source>`) |
| `--no-code` | Skip the replay with the code observer |
| `--threads` | Threads for the replay (default 8) |
| `--top` | Items printed per place (default 10); the reports keep all |
| `--out` | Where to write the reports, instead of `$GP_HOME/reports/gaps/<template>/` |

Each run writes `changes.json`, `interactive.json` (the catalog, [interactive elements](formats/interactive.md)), `interactions.json` and `drawn.json` to `$GP_HOME/reports/gaps/<template>/`, and a copy under `<template>/<recording>/` (the ID's first 12 hex digits), so the latest run is what the scorecard shows and each recording's results stay. `gp scorecard` reads the top items from these reports; it reads the counts from `gaps.<template>.*` metrics, which `gp gaps` here doesn't record.

## What it reads

Every detector reads the same facts about each frame, built from the recording and the template:

| Fact | From |
|---|---|
| RAM, and the bytes that changed | The frame trace's `ram` |
| Sprite objects on screen | The trace's `oam` one frame later, grouped into objects as `gp track` groups them ([tracks](formats/tracks.md)), with the sprite table entries of each |
| Background cells that changed | The trace's `nametables`, as 2x2 tile cells, with the scroll from `ppu.t` and `ppu.x` and the mirroring from `mapper.mirroring` |
| APU writes | The event stream's `apu-write` events; the sprite page from its `$4014` writes |
| The player, the mapped objects, the camera and the counters | The state mapping ([state mapping](formats/state-mapping.md)): the player's `entities.<name>[0].x` and `.y`, each `[slots]` slot whose flag is set (with its model entity when a `[[slots.kind]]` lists its ID), the first camera, and every `counters.*` field |
| In play | The mapping's `start` condition or its `slots.moving` condition holds (the game read its controller, for a mapping with neither), and the frame isn't a screen the player is idle on: the player's mapped position moved within 300 frames either way, and, with the code signals, the code that moves the player ran within 30 frames either way. That code is the routines making 80% of the play frames' writes to the player's position bytes, busiest first, so a title screen that borrows those bytes isn't it (`Run::refine_play`) |
| Code activity | A replay of the recording with the code observer (below), when the ROM is in `$GP_ROM_DIR` |
| What the model accounts for | Below |

What the model accounts for is a RAM byte the mapping reads (a field, or any slot's byte), or a name it cites. The cited names are every identifier in the template's `model.toml` (its facts' evidence quotes the disassembly's labels), `mapping.toml`, `fidelity.toml`, the mechanic catalog of `gp model mechanics` (component and hook records, `Ext::describe`) and the source of its hook crate. A labelled RAM byte or routine is explained when the model cites its label. The model's HUD rows (`present.hud`) account for the background cells under them, since the HUD draws the counters there.

Nothing in this is a game's: the facts come from the trace format every NES recording has and from the template's own mapping and labels, plus its model. A game with a trace and a state mapping gets every detector; labels name what is found, and the ROM adds the code signals.

## Code signals

Which code runs is as much a clue as what the screen shows: a routine that runs only when a block is hit is that block's handler, whether or not anything on screen changes. `gp_events::activity`, one of the gp-events collector's observers, watches every instruction of a replay through the same hook coverage uses (`Nes::step_with`), in the collector's one look at each instruction; `gp_coverage::activity::record` is the replay that drives it; like coverage it only reads the console, so the replay reaches the recorded frame hashes (checked every 600 frames and at the end of each segment), no runtime changes, and BEHAVIOUR_VERSION stays as it is. Replays run in segments from the recording's checkpoints, on several threads. For each frame it records:

- the routines entered, with how many times: every `JSR` target, the handler of each NMI and IRQ, and every label of the labels file (so code reached through a jump table or by falling through counts too);
- every write that changed a RAM byte, with the writing instruction's PRG offset and the routine it ran in, the top of a shadow call stack. `JSR` and interrupts push; `RTS` and `RTI` pop every entry the return's stack pointer leaves behind, so a routine that pulls its caller's return address and jumps (a jump table) doesn't put the stack out of step;
- every `$2007` write with the VRAM address it reached, and every APU register write, attributed the same way;
- calls to the routines it was asked to watch, with the caller and the A register.

Over the run it keeps each `JSR` target's call statistics: its calls and distinct callers, the values A had, and how many calls wrote the APU. It also keeps every instruction start that ran, for the static cross-check.

A routine's name is the label covering it (`Name`, or `Name+n` inside one). Without labels, an entry is named by its PRG offset (`sub_1A2B3`), so a game with no disassembly still gets its routines as clusters of code by entry point, and a write is attributed to the routine it ran in. Super Mario Bros.' labels file has RAM labels only, so in game-printer its routines came from the ld65 debug file of `just smb-listing` (neither is in gameprinter).

## The unexplained-change detector

`gp gaps changes` lists, in five places, what changes during play that the model doesn't account for.

| Place | What's listed | Ranked by |
|---|---|---|
| RAM | Bytes that change in play, aren't mapped and whose label (if any) the model doesn't cite, grouped by label: a label's bytes, and up to 8 bytes after it that have none of their own (`Label+n` grows the group), else a 16-byte block (`$05Ax`). The stack page and the sprite page are left out. Each group has its change counts, the share made in play, how much likelier a change is within two frames of a controller change (`input_lift`, 1 is none), its rank's parts (`score`, `event_lift`, `per_frame`, `engine_share`), the routines that wrote it and sample frames | How much likelier the byte starts changing near a game event than by chance (below) |
| Sprites | Sprite objects followed through the frames (as `gp track` follows them) that are in play and are near no mapped entity on most of their frames, grouped by palette and first tiles. Each has its tracks, frames, size, the share that start near a background cell change, the IDs of nearby mapped slots no model entity lists, and the routines that wrote its sprite table entries | Tracks |
| Background | Cells that change in play while visible, grouped by what they were and became (tile numbers). A frame with more than 40 visible changes is a redraw and a column or row with 6 or more is the scroll streaming in background, so neither counts, nor do the HUD rows. Each has its count, distinct world cells, the routines that wrote the cells through `$2007` and samples `[frame, world x, world y]`. The model's mechanisms that change cells at run time (hook code that calls `set_tile`) are named beside them | Count |
| Sound | The sound effects the game asks for that the model has no counterpart for ([sound requests](#sound-requests)), and the APU's channels with the game events their notes start with | Requests |
| Code | Routines that run in play whose label the model doesn't cite, with the unexplained RAM changes they made: mechanics with no visible effect show up here | Frames active, then changes caused |

### Ranking RAM

A byte that matters changes when something happens in the game; bookkeeping changes all the time. The RAM place ranks each group by its label's own bytes (the unlabelled bytes after it only when those never changed), and a byte by:

- its starts of change (a change after a frame without one) within a frame before to two after each kind of game event: each requested sound effect, each counter's changes (a counter that changes on over 5% of play frames is a clock and isn't an event), background cells changing at run time, and a mapped slot filling. For each kind, the starts that fell near one against the starts chance gives (the kind's share of play frames), smoothed by one count and weighed by `hits / (hits + 3)`; the best kind is the byte's `event_lift`;
- times the share of play frames it didn't change on (`per_frame` held down);
- weighted by the share of its writes not made by the sound engine (`engine_share`);
- divided down for a scratch byte that more than six distinct routines write (temporaries, pointers);
- scaled by `0.2` for a renderer's byte (30% of its writes made by a routine that writes the background through `$2007` (an update buffer it empties, a count it clears));
- scaled by `0.1` for a copy of the controller (90% of its starts on a frame the controller changed or the one after, clearly more than chance), and `0.2` for a byte set up with a level (80% of its starts within 10 frames of a background redraw or play starting or stopping).

## Sound requests

Raw APU writes say little: music plays all the time. What a game asks its sound engine for does: a coin sound, a bump, a stomp. Games ask in one of two ways, and both are found from the code signals:

- A queue: a RAM byte game code writes an effect ID to and the engine takes off. A byte is a queue when code at three or more places sets it to a nonzero value, it's cleared within three frames of most of those sets (at least 60%, and at least half of them by the engine), a note starts within three frames of at least 97% of the sets (clearly more than music alone gives; the share is held against the run's own rate), and it's set on at most half the frames. A byte with a sound name (sound, sfx, music, snd, song) needs only half its sets cleared (a third by the engine) and followed by a note. A byte set from three or more places that the engine takes off within three frames on 90% of its sets is a queue too, whether or not a note follows: a stop, a fade or a jingle that waits starts none. A queue of flags asks for the bits a write adds.
- A play-sound routine: a `JSR` target called from four or more places with three or more values of A that is either the engine's own way in (most of its calls write the APU) or feeds something that runs later (on at least half the frames it's called, a byte it writes is written by other code in the next three frames, as the engine takes the request off); or a sound-named one called from two or more places with two values that feeds. Either way it must be passed an ID: at least 40% of its callers load A with a constant just before the call (`lda #id` then `jsr`), and when it feeds, most of the IDs it's asked with show up in what it writes. A boss routine that happens to call the sound routine, or a helper that always asks for one fixed sound, is called with whatever A held and writes no ID, so it is no interface. The queue the helper writes is one. A routine the engine calls too (over 5% of its calls) is a utility, such as a bank switch, and one called more than once every four frames is a renderer; neither is a way to ask.

The sound engine is the code under a routine that writes the APU on most of its calls, and the parts of PRG the APU is written from on a tenth of the frames or more (8KB banks on a banked board, 1KB pages otherwise; start-up code that silences the APU once isn't the engine). Its writes and calls aren't requests. A routine call the engine makes in the frame a queue was set is the engine taking the request; a queue a play-sound routine fills is the same requests seen from inside it, and the routine's are kept since they name the caller.

Each request is an event. It records the frame and interface with the effect ID. It also records the routine that asked and a name. The name is a constant of the disassembly (an ld65 debug file's equates) with that value and a sound name that the source uses within three lines before the interface's name (`lda #Sfx_Bump` then `sta Square1SoundQueue`), else `<interface> <id>`. A constant of the same value used elsewhere is not taken as the name. The constants come from an ld65 debug file or an assembler listing that keeps each source line after a fixed column of address and bytes (asm6f's `.lst`: every `Name = value` line, and the names each use is followed by within three lines); with no `--listing`, one is looked for beside the labels file (its folder, `listing/` or `build/`).

An interface whose effects no constant names (one routine asking by number) gets names from the sound engine's pointer table: a table of two-byte pointers in the engine's code windows, read by one of the engine's indexed loads (`lda table,y` and the like), whose entry for each ID points at the start of a ROM label (the labels file's code or data labels) for at least eight of the IDs and 90% of them. The read requirement keeps a table read one entry off, whose entries point at labels too, from being taken. A queue of flags indexes no table.

Each effect is classed (`class` in `changes.json`): `music` when it's asked for within a second of a scene change (a background redraw, play starting or stopping) on most requests, or when the engine copies the ID into a byte of its own that keeps it for 300 frames or more (the music now playing); `effect` if that byte keeps it for less; `command` where the engine keeps it nowhere and fewer notes start after it than before (a stop, a fade). One interface that asks for both, with music in one range of IDs and effects in another, gives every ID its range's class when 80% of the classed IDs agree on a split.

In `changes`, each effect the model cites no name of is listed with its count, the routines that ask for it, sample frames, the counter changes within 12 frames after it (each counter's most common change and the share), the mapped objects that appear within four frames, the share near a background cell change, and the strongest of those that beats chance (a counter's change must happen at least twice as often after the effect as in any 13 frames) as `suggests`: a hint at the behaviour the effect marks. In `interactive`, the requests at a trigger are part of its evidence, and a request with no cell change near is an interaction of its own, which finds the reactions that change nothing on screen.

## The interactive-element cataloger

`gp gaps interactive` finds the parts of a level that react, and catalogs them in the format of [interactive elements](formats/interactive.md).

An interaction is found three ways: a background cell that changes at run time (as the background place counts them); a sound request with no cell change within two frames; or a handler routine firing with neither. A handler is a routine that fires on at most 5% of play frames, at least twice, and has a cell change the player is by (not `away` on the frame before) in its frame or the two after on at least 30% of them, so a menu's or a scroll's cells don't make their writers handlers. An interaction found by its sound names the code that asked for it (`requester`, the label around the call or the queue write), and the catalog lists those (`requesters`): the handlers of interactions that change no cell, such as a stomp or a pickup (and a hit).

A cell kind the player was away from on 90% of its changes, with no sound asked for on 30% and no counter moving on half, is scenery nothing touched, such as an animation or a scroll seam, a menu or a screen of text. Those kinds go to the catalog's `scenery` list, not its `kinds`.

Each interaction gets its context:

- the player's box (the sprite object nearest its mapped position, else a 16 pixel square) relative to the cell on the frame before (below, above, left, right, inside or away), and its speed over the two frames before;
- the counters' changes from 2 frames before to 12 after;
- objects that appeared within four frames and 48 pixels: a mapped slot's model entity, a slot whose ID no entity lists, or a sprite object no mapped entity explains (a coin popping, brick pieces);
- APU notes started within three frames, and the sound effects requested;
- the handler routines that fired within two frames before, and the primary one: the most tied to cell changes, then the rarest;
- rarely changing RAM bytes (under 2% of play frames) that changed with it, by label: a level buffer's cell, a block's timer;
- what the cell became, and whether it changed back within 60 frames (a bump).

Interactions cluster into kinds by the cell's tiles before and after, or by the effect or handler for those with no cell change. A kind's record has its trigger (the most common relation, speed, handler and effect), effects (each counter's most common change, what spawns, notes and effects, RAM), result (what the cell became, how many changed back), limits (cells, the most changes one cell had within 1,200 frames of each other, the interval between them) and up to five evidence frames.

### Cross-check with the code

For a kind whose trigger routine (its primary handler, else the code that asked for its sound) has a label, its code is read from the ROM by control flow from the label (`code::body`, as in [scoring](#scoring)), across the labels a disassembly puts at every branch target, or the label's first 512 bytes when the walk finds nothing: every branch, jump and call target in it is checked against the instructions the replay ran. Targets that never ran are the cases the code handles that no recording exercised (`code.unexercised`), and the immediate values it compares with (`code.compares`) are usually the tile or object classes it branches on. A target is followed when it's in the routine's 8KB window, or anywhere on a board with 32KB of PRG or less.

## The drawn-box check

`gp gaps drawn` places each mapped object's sprite where the model's presentation draws it and compares that with the original's sprites. The model's box is what `gp_present::Animator::scene` gives the web preview for the base picture of the look of the state the entity's lives start in (its machine's `initial`, else its first look): a frame laid out from the position (a captured one, `Present::origins`) at its origin from the body, any other with its left at the body's x and its bottom on the body's bottom edge (`y + height - picture height`), minus the camera. The body is placed at the original's mapped position, so the check measures where the model would draw the frame if its body were where the mapping says; whether the model's body is there is `gp interp slots`'s y line. The original's box is the nearest sprite object on screen within 24 pixels (left plus top difference), each object paired once a frame. Per entity kind it reports the frames drawn and matched, the mean and spread of the difference in left, bottom (`dy`), width and height (model minus original), each one's median and the share of frames within a pixel of it, and the frames with the largest vertical difference. The vertical difference is taken at the bottom edge, where a sprite stands, since a game's frame may keep blank rows at its top in the sprite table (Super Mario Bros.' goomba frame is three rows, the top one blank), which only the height difference shows. A difference is systematic when its median is at least a pixel and it holds within a pixel on 60% of the matched frames, which pairing mistakes (two enemies close together drawn as one object) don't explain. Report only. Each kind's offset is also measured as a detected fact (`offset`, `offset_share`): the medians of the left and bottom differences over the frames whose original object has the kind's usual size and isn't near another mapped object's box too, so two objects drawn as one (`composite_frames`, left out) don't move it, and the share of those frames within a pixel of it.

On Super Mario Bros. with captured frames (2026-10-29, bd0e4aec and c8f4e908), the goomba, both koopas, the buzzy beetle, Mario, the piranha plant, the hammer brother and the large lifts are 0 vertically, within a pixel on 82% to 100% of their clean frames; before the origin and the bottom edge were used, the goomba and buzzy beetle read -1 and the koopas +7. What is left is the 8 pixels of blank rows (`dh -8`) and a pixel of x on the koopas' and paratroopa's frames, which the check doesn't mirror for facing. The small lift (+2) and the kinds without a captured frame (bullet bill and the grey cheep cheep, -9) remain too.

## How each detector generalizes

| Detector | Needs | Without it |
|---|---|---|
| All | A recording with a frame trace (`gp record` keeps one), and the template's model and state mapping | |
| RAM, background, sprites | The trace and the mapping | |
| Names | A labels file, or a ld65 debug file | Addresses, 16-byte blocks and `sub_<offset>` routines |
| Code, sound requests, handlers, write attribution, the cross-check | The ROM, for the replay | Those parts are empty and the report says why |
| Sound effect names | A debug file with the disassembly's constants | `<interface> <id>` |
| Drawn | The lifted assets, for the presentation | The report says why |

The rules are thresholds on what any NES game does: a frame boundary, a sprite table copied by DMA, background in nametables, an APU written by a sound engine, a call stack. None reads a game's address or constant.

## Scoring

Each detector is held to a labelled reference set per game: `crates/gp-gaps/truth/<template>.toml`, built from the pinned disassembly and what is known about the game. A set holds names and the IDs of the disassembly's constants only, never game data:

| Section | What it names |
|---|---|
| `[sound]` | The interfaces the game asks for sound through (RAM queues, play-sound routines), which are music, and every effect as interface, ID and constant name, with music and engine commands marked |
| `[[element]]` | Each interactive element and game event: the routines that handle it and the sounds it asks for |
| `[ram]` | The RAM variables that carry behaviour (`matter`), and the per-frame bookkeeping and sound-engine internals that don't |

`gp gaps all` scores a run against the template's set (or `--truth FILE`) when it has the code signals, writes `score.json` next to the other reports. (game-printer also kept each precision and recall as a metric for its scorecard.)

| Detector | Found items | Precision | Recall |
|---|---|---|---|
| `sound` | The sound interfaces | Found interfaces the set names | Set interfaces the game used (a queue written, a routine called) that were found |
| `effects` | Every requested effect, by interface and ID | Found effects the set names | Set effects the game asked for (the queue written with the ID, the routine called with it) that were found |
| `interactive` | The catalog's kinds | Kinds whose trigger routine or requested effect belongs to a set element | Set elements exercised (a routine ran in play or a sound was asked for) that some kind matched |
| `handlers` | The catalog's handler routines | Handlers inside a set element's routine | Set routines that ran in play with a handler found inside them |
| `ram` | The top 25 items of the RAM place | Items the set lists as mattering | Mattering items in the place that made the top 25 |

A found routine belongs to a set routine when it is the routine or lies in its code. The code is followed by control flow from the routine's entry (`gp_gaps::code::body`): branches and the instruction after them, a `JMP` within the routine's 8KB window that isn't another routine's entry, and the instruction after a `JSR` unless the callee pulls its return address (a jump table follows). Labels at every branch target, as the Mega Man 2 and Super Mario Bros. 2 disassemblies have, don't split a routine.

What the recording exercised is read from the code signals, not from the detector being scored, so a detector that finds nothing scores a recall of 0, not 1. Names in a set that no label has are listed in the report (`unknown`).

## Rounds

The rounds were scored on Super Mario Bros., Mega Man 2 and Super Mario Bros. 2, whose reference sets aren't in gameprinter (other games' code was cut in the migration); Moss Hop's set is. They are kept as the record of why the detectors are as they are.

Detection is refined in scored rounds: each round picks the worst score or the biggest known problem, changes the detectors in a way that holds for any game, and scores all three games again. The scores are on Super Mario Bros.' c8f4e908, Mega Man 2's 35a900f9 and Super Mario Bros. 2's c31a0471, each `sound`, `effects`, `interactive`, `handlers` and `ram` as precision/recall, then effects named.

| Round | Change | Super Mario Bros. | Mega Man 2 | Super Mario Bros. 2 |
|---|---|---|---|---|
| 0 | The reference sets and the scorer; no detector change | 0.83/1.00, 0.79/0.96, 0.88/0.83, 0.69/0.62, 0.00/0.00; 0.67 | 0.25/1.00, 0.38/1.00, 0.20/0.75, 0.00/0.00, 0.00/0.00; 0.00 | 0.80/0.67, 0.89/0.78, 0.14/0.89, 0.20/0.24, 0.00/0.00; 0.00 |
| 1 | RAM ranked by its starts of change near game events, per event kind, held down for per-frame, sound-engine, scratch, controller and level-setup bytes (it ranked by change count) | ram 0.52/0.52, 2 bookkeeping in the top 25 (was 24) | ram 0.32/0.36 (15 -> 14) | ram 0.44/0.44 (25 -> 8) |
| 2 | Sound interfaces: a play-sound routine must be passed an ID (callers load A with a constant; its writes carry the IDs); a byte the engine nearly always takes off is a queue without a note; the engine's code is where the APU is written on a tenth of the frames | sound 1.00/1.00 (0.83/1.00), effects 1.00/1.00 (0.79/0.96) | sound 1.00/1.00 (0.25/1.00), effects 0.95/1.00 (0.38/1.00) | sound 1.00/1.00 (0.80/0.67), effects 1.00/0.97 (0.89/0.78) |
| 3 | From the point-and-tell lane's first results: a box question answered by the cells in the box at the frame nearest the request, the rest nearby (`focus::attribute`); counters in the units the player sees (the HUD's `divide`); the same moment matched by position (`align`). The MM2 skew effects joined its set | unchanged; brick score +50, coin +200 (were +5, +20) | effects 1.00/1.00 (0.95: three effects the set lacked) | unchanged |
| 4 | Cell kinds nothing touched go to `scenery`; handlers are tied to cells the player is by; sound-found kinds name their requester, which the handler score counts. The sets gained the flagpole slide, the bullet bill and the boss hit routines | interactive 0.87/0.83 (0.95/0.83: the balance platform's rope is now scenery), handlers 0.61/0.83 (0.69/0.62) | interactive 0.75/0.77 (0.20/0.75), handlers 0.20/0.36 (0.00/0.00) | interactive 0.56/1.00 (0.18/0.94), handlers 0.38/0.76 (0.20/0.24) |
| 5 | Effect names from asm6f listings and from the engine's pointer table; effects classed as music, effect or command | named 0.68, classed 0.81 (0.00) | named 0.91 (0.00), classed 0.68 (0.00) | named 0.71 (0.00), classed 0.68 (0.00) |
| 6 | In play needs the code that moves the player to run (title screens, menus and text screens the mapping's condition holds on are out): SMB2 66,220 -> 55,723 play frames, MM2 8,431 -> 6,227, SMB 61,540 -> 60,747 | unchanged | interactive 0.82/0.69 (0.75/0.77: the stage music asked before play starts is out), handlers 0.29/0.36 (0.20) | handlers 0.47/0.76 (0.38), ram 0.48/0.48 (0.40) |
| 7 | The code cross-check reads a routine by control flow and covers sound-found kinds through their requester; RAM holds a renderer's bytes down. No score moved; the cross-check reads more of each routine | 34 kinds checked, 75 targets (25 kinds, 28) | 12 kinds, 57 targets (none) | 38 kinds, 187 targets (17, 11) |
| 8 | The drawn check measures each kind's offset on frames with the kind's usual object size and no second mapped object near (composites left out) | goomba 0, -1 on 93% of 3,809 clean frames (3,486 composite frames out); koopas +7, bullet bill -9 | needs `gp sprites capture` | needs `gp sprites capture` |

After round 8, the other store recordings (same detectors and sets; short recordings exercise few elements, so their recall is low and their RAM top is mostly what one level sets up):

| Recording | sound | effects | interactive | handlers | ram | named | classed |
|---|---|---|---|---|---|---|---|
| Super Mario Bros. 782c557d (warpless) | 1.00/1.00 | 1.00/1.00 | 0.94/0.70 | 0.67/0.83 | 0.56/0.56 | 0.64 | 0.73 |
| Super Mario Bros. bd0e4aec (warps) | 1.00/0.80 | 1.00/0.70 | 0.96/0.81 | 0.66/0.77 | 0.36/0.36 | 0.86 | 0.75 |
| Super Mario Bros. run-right 8a476c6a | 1.00/0.67 | 1.00/0.67 | 1.00/0.13 | 0.00/0.00 | 0.00/0.00 | 0.50 | 0.75 |
| Mega Man 2 c8d6fd36 | 1.00/1.00 | 1.00/1.00 | 0.67/0.83 | 0.30/0.40 | 0.36/0.39 | 0.91 | 0.67 |
| Mega Man 2 Wood Man baseline 0be73728 | 1.00/1.00 | 1.00/1.00 | 1.00/0.33 | 1.00/0.40 | 0.20/0.63 | 0.00 (under eight IDs, so no pointer table) | 0.55 |
| Super Mario Bros. 2 c639b478 (enter 1-1) | 1.00/0.80 | 1.00/0.88 | 0.50/0.17 | 0.00/0.00 | 0.00/0.00 | 0.71 | 0.71 |

## Aligning by position

The model's player leaves the original's path within seconds of the same input, so comparing the two at one frame number soon compares two places. `gp_gaps::align` matches a moment by where the player is instead: on the same level when both runs name one, the nearest position (x plus half of y), then the nearest frame. `drift` gives the distance between the players at the same frame and the matched frame with how many frames apart it is. `gp serve`'s pause and point (`crates/gp-cli/src/serve/pointing.rs`) reports it as `alignment` (point and tell). The original's level isn't read from its RAM yet, so the match there is by position alone.

## Limits

- The background is read as 2x2 tile cells of the nametable. A game with another cell size, or one that draws level objects as sprites, shows them in the sprites place instead.
- Cells under a status bar the model has no HUD for are counted as level cells; a scroll split mid-frame (a status bar) puts the playfield's scroll on every cell.
- A model run of the same input is not compared cell by cell: the model's player leaves the original's path within seconds, so the background place lists every runtime change and names the model's tile-changing mechanisms beside them (filed in docs/open-questions.md).
- Sound effect names need a listing with constants or a pointer table the engine reads; a flag queue's effects with a value shared by other constants (Super Mario Bros.' `Square1SoundQueue 0x01`) stay unnamed.
- Pairing an original sprite object with a mapped object is by distance. Enemies close together are one object, and the spread of a kind's differences shows it.
- The RAM place's rank can't tell a renderer's buffers (a VRAM update buffer, a PPU register's copy) from game state when they change with background cells: both start changing with the event.


## On an arcade recording

`gp gaps changes|interactive|all` read an arcade recording too (port wave 6b, `crates/gp-cli/src/gaps_arcade.rs`): the frame facts come from gp-arcade. RAM is the main CPU's address space (a mapping's addresses read as they are; changes are the work RAM, E000-E7FF, named from there: gp-gaps' `Run::ram_base`); objects are the sprite list clustered as gp-track clusters it; background cells are video RAM's 32x30 tiles, with the routines that wrote them; sound is the AY-8910 writes; code is a replay on the main Z80 with the access log (routines entered, RAM writes with their instruction and routine, and the routines that write the sound board's latch as the engine). `gp gaps drawn` reads an arcade game's boxes from its arcade lift (port wave T, `gaps_arcade::model_boxes`), since the NES box reader reads CHR tiles and an NES map: each kind's sprite in the state its lives start in, a captured frame of the map's `sprite-set` (from `gp sprites capture`) laid out from the position with each piece a whole MCR sprite, 16 by 16 in the game's units, or tiles of an `mcr-sprites` slot `width` to a row, stood on the body's bottom edge. The mapped positions are placed where the board draws a sprite with those bytes, `(x - 3, 241 - y)`, the sprite list's units. A kind with no sprite isn't checked, so a model with no captured sprites checks no kind.

On Tapper (a five-level play, frames 1000 to 2500, with games branch tapper-2's model, mapping and labels): 36 RAM groups, 39 sprite kinds, 9 background changes and 90 routines the model doesn't cite, in 5.5 s; the labels name what they cover (`level_ram`, `customer_records`) and the rest by address (`$E7Ex`, written by the frame interrupt's handler $00D8 and others); the background changes are one tile column redrawn by $05D5.
