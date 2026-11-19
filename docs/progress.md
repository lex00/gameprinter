# How far along a game is

`gp progress` says how far along printing a game is, for someone who started from a ROM and nothing else. It is a ladder of nine stages, from the ROM to the whole game played identically on the rebuilt game. Each stage has one number from 0 to 100 and a line that says when it is done. The owner asked for it on 2026-10-30, in game-printer: "think about what metrics you can use to tell someone how far along they are in this process with a rom they started from scratch". It came to gameprinter on 2026-11-13, and works on an arcade romset too.

```
gp progress --template donkey-kong --run donkey-kong [--measure --module donkey-kong.wasm] [--json]
```

| Where | What |
|---|---|
| `gp progress --template GAME [--run RUN] [--json]` | The ladder from what exists: the game's folder in the games checkout (or a folder holding model.toml), the run's model and scores, and the last measure. Reads only. A run is gp serve's: a game by its folder's name, whose model is its working copy (`$GP_HOME/work/serve/GAME/model/`) once there is one |
| `gp progress ... --measure` | Also measures the heavy stages (below) and keeps the result in `$GP_HOME/work/progress/<game>/<run>.json`. `--playthrough ID` picks the full-game recording (or a baseline file); without it, the longest store recording of the game's ROMs. `--no-clips` and `--no-whole` leave parts out and keep what the last measure had. `--module` gives the game's module, for a model with hooks. `--no-save` shows the ladder with this measure and keeps nothing, so a trial (a short `--playthrough`, say) leaves the kept measure as it was |
| `gp progress ... --write-ignore MAPPING` | Writes the last measure's RAM roles into a mapping file as its `[[ignore]]` list ([the ignore list](#the-memory-maps-ignore-list)). NES only: the roles come from a trace of the 6502 |
| `gp progress --next [--template GAME] [--json]` | [What next](#what-next): where the port stands and the one next step, for the person and their agent |
| `gp progress --next --copy NAME [--json]` | The same for one of the person's [copies](#copies-and-the-other-journeys): a theme they are making or a new game, by its name, or `theme/NAME` or `new/NAME` |

The code is `crates/gp-cli/src/progress.rs` (the ladder, from its facts alone, so the tests give it made-up inputs) and `crates/gp-cli/src/progress_measure.rs` (the measure). The start and the lockstep from it are the jumpathon's (`crates/gp-cli/src/sweep.rs`, `measure`).

## What next

`gp progress --next` answers "where are we, and what now?" for a person porting a game with their agent. The same answer is the status line under the page's header on every tab (`GET /api/next/GAME` on `gp serve`) and the MCP tool `next` ([mcp.md](mcp.md)). The owner asked for it on 2026-11-13: "i want a development loop that keeps prompting the person about what to do next, gives them clues if they seem lost, etc". The design is guided-loop.md in the owner's notes; this is its smallest first version, for porting on NES and arcade.

It reads files and records only: the game's folder, the quest log and its sessions, the gate's scores (`$GP_HOME/work/refine/GAME/status.json`) or gp serve's background scores, the last `--measure`, and a published record (`$GP_HOME/work/published/GAME.json`, with its `link`; nothing writes it yet). It runs no score, ROM or build, so it answers in well under a second and a new conversation gets the same answer. It suggests and never starts work. Without `--template` it takes the game whose page was opened last, or the only game; with no game at all it reads the ROM folder and asks which file to bring over.

The answer (`version` "next 2"; 2 added `steps` and `copies`):

| Field | What |
|---|---|
| `stage` | One of twelve: P1 no game file, P2 a game file and no game, P3 finding the player, P4 first clips, P5 clips not in the game's tests yet, P6 scoring, P7 fixing the model, P8 play only the person can give, P9 a full run not checked, P10 the full run parts, P11 ready to go online, P12 online. P0 asks which game when several are here and none was opened |
| `stands` | Where the work stands, in a sentence or two of plain words |
| `next` | The one next step: `who` (`tools`, free and unattended, which the agent starts without asking; `agent`; or `person`), `says`, `how` (the page's button, the command) and `act` (what the page's button does) |
| `also` | Up to two other steps, the person's first when `next` isn't theirs |
| `running` | What the background is doing and when to look again |
| `waits_for_yes` | Steps that cost money or make something public (putting the game online), which wait for the person's own yes; and a picture the agent proposes for the person's theme (`what: "picture"`), open or answered and not yet read by the agent ([sprite-editing.md](sprite-editing.md#a-picture-of-your-own-theme)) |
| `clue` | A clue when the person seems lost, or null |
| `facts`, `text` | The counts it read, and all of it as the terminal prints it |
| `journey`, `subject` | `port`, `theme` or `new`, and the game or copy the answer is about |
| `steps` | For a copy: its six steps, each `{n, id, name, who, state}` with `state` `done`, `now` or `later`. Empty for a port, whose stages are above |
| `copies` | Every copy the person has, each `{journey, name, from, touched_ms, stage, who, says}`, newest first, so an agent arriving cold sees all of them |

The next step is the first of these that applies: something that stops all work (the game's file isn't found, the background stopped on an error); a step waiting for a yes; a question only the person can answer (the agent's question on CLIPS, a question on REFINE); a hand-off the agent hasn't answered; free work for the tools that isn't running (triage when no row was tried, scoring, the full-run measure); the agent's work (map the player, add new clips to the game's tests, the clip that goes wrong first, why a full run parts); play only the person can give (the first NEEDS YOU row, with its request); the stage's own step.

The clues, at most one at a time, never while recording or reviewing, and each at most once in 10 minutes (the page keeps the count; after the third time on one signal the words offer another way in):

| Signal | Seen by | When |
|---|---|---|
| `idle` | The page | No button for 2 minutes on CLIPS while a row only the person can play is open |
| `drops` | The page | The same row's recording dropped or too short three times in 10 minutes |
| `hops` | The page | Four tab changes in 2 minutes with nothing done |
| `away` | The page | The agent's question waiting 5 minutes while another tab shows |
| `question` | The files | A question on REFINE open for 10 minutes while nothing else runs |
| `paused` | The page's server | The background paused (it pauses at every server start) with clips to score |
| `words` | The files | A note typed on the page that expects a download, an install, an emulator or a new ROM: the answer is what Game Printer makes |

Not yet: a handed-off clip that fails its row's goal (needs the triage's test on the server), times on turns and clips (an unanswered question's age), the person's words in the conversation (the agent sees those), and a LATER button. gp serve keeps whether its background is on in the state file's `live` (with its process), so the terminal and the agent see it too.

## Copies and the other journeys

Porting a game is one of three journeys. The other two make a copy: a theme for a game, or a new game from nothing or from a template (issue #5). The page's first screen (`/start/` on gp serve, [mechanic-page.md](mechanic-page.md#the-first-screen)) starts both and lists every copy with CARRY ON; `gp progress --next --copy NAME` and the MCP tool `next` with `copy` give the same answer as for a port, with the copy's six steps and whose turn each is. The code is `crates/gp-cli/src/journeys.rs` (the copies, the facts and the answer, from files only) and its tests in `journeys/tests.rs`.

| Copy | Where it is | Made by |
|---|---|---|
| A theme | `$GP_HOME/work/themes/NAME/`, holding `theme/look.txt` | `gp theme look new`, from the page's MAKE A THEME FOR A GAME or by the agent |
| A new game | `$GP_HOME/work/mcp/NAME/`, holding `model.toml` | `template_fork`, from the page's MAKE A NEW GAME or by the agent |

What a journey keeps of its own is under `$GP_HOME/work/journeys/KIND/NAME/` (KIND `theme` or `new`), never in the copy: `journey.json` (where it came from, the person's style words, the steps the person marked done), `check.json` (the last `gp theme look check` of the theme), `play.json` (the theme last seen in the game), `verify.json` (the last `gp level verify` of the new game's stage), and `do-WHICH.json` (a page button's last run). The commands write their part whoever runs them, the page or the agent, so the answer is the same either way. Without `--template` or `--copy`, the copy opened last on the page answers when it was opened after the last game.

A theme's steps:

| Step | Whose turn | It is the step while | The page has |
|---|---|---|---|
| T1 Pick the game and a style | You | The theme doesn't say which game, or has no style words | The style box |
| T2 Make the pictures | Your agent | It has no pictures | What to say; the folder for the person's own pictures |
| T3 The look check | The tools | It changed since its last check, or the check found problems | CHECK MY THEME and the findings in words |
| T4 See it in the game | You | It passes and wasn't played since | SEE IT IN THE GAME (the game's screen on gp serve with the theme picked, for a game of the games checkout; `gp theme look play --background` for a theme of a new game or a template), PICTURES OF IT IN THE GAME (`--shots`) when the ROM is here or the game has pictures of its own |
| T5 Change pictures | You | Until the person presses READY TO SHARE | BESIDE THE ORIGINAL (`gp theme look play --shots DIR --against original`): each moment of the game's route in the theme and in the game's own pictures, side by side, stepped through with BEFORE and NEXT; and the pictures the agent proposes, on their cards |
| T6 Share it | You | Then | The share link and the publish request, shown to send by hand: nothing is published from the page |

A new game's steps:

| Step | Whose turn | It is the step while | The page has |
|---|---|---|---|
| N1 Start from nothing or a template | You | (done once the copy exists) | Nothing, the first choice (`gp game new NAME --empty`: one stage of floor, a player that walks and jumps, a goal, its own plain pictures and no sound), then each template and what a template is |
| N2 Name it | You | (done once the copy exists) | Its folder |
| N3 See it play | The tools | Its pictures and levels aren't there, or it was never built to play | BUILD IT TO PLAY (`build_publish`) and PLAY IT |
| N4 Change a stage | You | Until the person presses DONE WITH THE STAGE | SHOW THE STAGE (`gp level render`), CHECK THE STAGE (`gp level verify`), UNDO THE LAST CHANGE (`gp level undo`) |
| N5 Give it a look | You | Until DONE WITH THE LOOK or SKIP | MAKE A THEME FOR IT: the theme journey on the new game |
| N6 Play and share | You | Then | PLAY IT, and MAKE A SHARE LINK (`gp game share`): the game in the person's own registry on this computer, with the themes made for it, and the link that plays it, `/share/p?g=local/NAME&look=ID` |

Not there yet, and said so on the page: a new game's share link opens on this computer only, until the person puts the page and their registry online together (the page says so in one sentence); and CHECK THE STAGE on a stage `gp level verify` can't play to its end answers NOT CHECKED with the reason (docs/level-kit.md, "Editing a stage", says which kinds it plays; Moss Hop's tile stage is one).

## The ladder

| Stage | The number | Done when |
|---|---|---|
| 1. Source | Of four checks: registered in the game's `roms.toml` (or built from source with a hash `template.toml` accepts), verified, the ROM found in the ROM folder by its hash, and the game's `labels.toml` | All four |
| 2. Memory map | The share of the game-state RAM bytes the game changes in a full playthrough that the state mapping names: the bytes it changes less those the mapping's ignore list leaves out. A byte the mapping names (a field, a slot table, or its `[cells]` level grid) is game state whatever the list says | Every game-state byte is named |
| 3. Objects | The share of the object kinds a full playthrough shows (each slot table's ID byte in a live slot) that the model has. Also: how many of those a scored clip measures | Every kind is modelled and measured in a scored clip |
| 4. Levels | The share of the game's levels (the mapping's slot levels, else the model's) the model has. The starts the rebuilt game can begin at aren't counted in gameprinter yet | Every level is in the model and every start can begin |
| 5. Mechanics | The share of the model's facts whose evidence cites the game's code. Also: the open missing-mechanic findings (unanswered questions that carry one, a part counted once) | No finding is open |
| 6. Clips | The share of the code addresses a full playthrough runs that the game's corpus clips run on the original, each from its start. Also: the mutants a kept clip catches, of those the corpus names | The clips run all of it, and every mutant is caught |
| 7. Behaviour | The share of the clips gp serve's background work has scored that are exact. Also: behaviour fidelity and frames together | Every scored clip is exact |
| 8. Picture and sound | Picture fidelity, once gp serve keeps a screen score (it doesn't yet). The picture-gap groups (`gp picture`) and sound aren't counted yet | No picture-gap group open |
| 9. Whole game | The rebuilt game started from the original's state at the playthrough's first frame in play (the start translation, carrying every loaded object), then the full playthrough's input on both in lockstep: the frames before the first frame any measure parts, as a share of the frames compared. Also: level by level, each from the original's state at that level's first frame in play to the next level's, the frames before its first part (a table, and their sum as a share); and the frames on which every measure agrees when the rebuilt game is put back after each part (resync mode) | 100%: the whole playthrough plays identically |

A stage that can't be computed from what exists says "not measured" and gives the command that measures it. Numbers are rounded down, so 100 means all of it.

"In play" means the frames the player controls: `observe.toml`'s `control` when the model has one, else the mapping's `start`, else every frame the game reads the controller. On the NES the stack page (`$0100-$01FF`) and the sprite page (`$0200-$02FF`) are left out of the RAM count. The playthrough is the longest store recording of the game's ROMs unless `--playthrough` names one.

## On an arcade romset

The original is gp-arcade's board, through the platform seam (`OriginalRuntime`), and every stage reads the game's own mapping. The RAM counted is the board's 2 KB of work RAM at `E000`, by its index from there (a mapping or label address `E123` is byte `$0123` of the count); nothing is left out of it. The code is the main Z80's instruction addresses from gp-arcade's trace (the features `gp coverage` reads, `crates/gp-cli/src/covered.rs`). The RAM roles come from the main Z80's trace and access log (`crates/gp-cli/src/ramrole_z80.rs`), classified by `gp_nes::ramrole`'s own rules: a call is a `call`, `rst` or an interrupt's handler, returned once the stack pointer is back; the controller is read by an `in` from ports 00 to 02 (the coin and start and the sticks, not the dip switches at 03 and 04); a sound write is an `out` to the sound board's latches (1C to 1F); a picture write is one to sprite, video or palette RAM (E800 on); the stack's own accesses aren't counted. `--write-ignore` writes each entry at its E000 address.

## The memory map's ignore list

Many RAM bytes the game changes hold nothing a model needs: the controller's bits, a frame counter, a value a routine works with and drops, the sound driver's state, a pointer. The mapping's `[[ignore]]` list (schema 21, docs/formats/state-mapping.md) names them, each with its role and why, and stage 2 counts the rest. The list is written by a deterministic classifier, not by hand: `--measure` traces every instruction of the playthrough on the original (`gp_nes::ramrole`), and `--write-ignore` writes what it says. This is the NES's. The rules apply in this order.

| Role | Rule |
|---|---|
| `sound` | Only the sound routines touch it; flagged for review when the game writes it and only they read it (a sound request). A sound routine writes an APU register, directly or in a call, and never a PPU register; a routine only sound routines call is one too. Calls are read from `JSR`, the interrupts, `JMP ($nnnn)` and the stack pointer, and the labels' routines name them |
| `input` | Written by a routine that reads $4016 or $4017 and writes no PPU or APU register, or its value is such a byte masked on at least 99% of the frames it is written (flagged below 100%), with the controller's bits varying |
| `counter` | At least 99% of its changes are +1 (or -1), it changes on at least half the frames it runs (a pause stops one), and it wraps at least twice |
| `scratch` | No frame reads it before writing it, or (flagged) at most 1 in 100 frames do, or what a frame reads first is always one value the frame before left. Written and never read is scratch too, flagged |
| `pointer` | Scratch that the code uses as a pointer (`($nn),Y`, `($nn,X)`, `JMP ($nnnn)`) or loads into X or Y to index with, on a quarter of its reads or more |
| `state` | The rest: live across frames |

A byte the mapping names never goes in the list. The stage also says how many bytes the list leaves out by role, how many are flagged for review, and when the last trace and the list disagree. Write the list again after the mapping or the playthrough changes.

## The whole game from the original's state

The rebuilt game starts from the original's state, translated at the first frame in play the playthrough reaches, as a start does (docs/interpreter.md, "Many starts"), so a value the model's power-on state holds differently from the original no longer parts on the first frame. Each level runs the same way from its own first frame in play, to the next level's start. The one pass on the original keeps a copy of it at each start, so no run replays the frames before its own. gameprinter runs them one after another; game-printer ran them side by side on the pool. Two measures of the same model give the same numbers.

The levels are the mapping's slot levels as the playthrough meets them, so a level played twice is two rows, and an area the level conditions read as another level is a row of its own.

## Donkey Kong and Tapper, 2026-11-13

Donkey Kong: Original Edition, against `dk-clear-25m-arcsprites` (the first stage cleared, 3,674 frames), with the game's module, in 4 min 38 s on a debug build (most of it the 114 corpus clips on the original). game-printer's `gp progress` on the same recording and model gives the same RAM, object kinds, code, levels, roles and whole-game numbers.

| Stage | Number | What it says |
|---|---|---|
| 2. Memory map | 28% | 116 of 411 game-state bytes are named; the trace says 241 more aren't game state |
| 3. Objects | 100% | 5 of 5 kinds the playthrough shows are modelled |
| 6. Clips | 100% | The 114 corpus clips run all 3,799 code addresses the playthrough runs, and 2,987 it doesn't |
| 9. Whole game | 38% | 1,419 of 3,671 frames play identically from the original's state at its frame 4, then they part at frame 1,423 on the third barrel's tiles and place; put back after each part, 3,618 of 3,671 agree (98%) |

Tapper (the Budweiser set), against `tapper-five-levels-arcsprites` (10,933 frames), with a module built from the games checkout, in 143 s without the clips: 11 of the 1,255 work RAM bytes the game changes in play are named (the mapping names the bartender and the score); the whole game plays identically for 866 of 10,839 frames from the original's state at its frame 95, then parts on the player's place across and height at frame 961; put back after each part, 10,758 of 10,839 frames agree (99%).

## Limits

- The RAM count is the bytes that change. A byte written with the value it had isn't seen.
- The ignore list's rules read one playthrough. A byte the playthrough never reads may be read elsewhere (it is flagged), and a short one may not wrap a counter twice.
- The frame boundary is the PPU's vblank start. A game that does its work in the NMI keeps a frame's work in one frame, except on a lag frame (the 1-in-100 slack for scratch); a game that works in its main loop across the boundary would show more live bytes.
- The NES powers on with RAM cleared (`PowerOn::default()`), as the lockstep does. A game that seeds from uninitialized RAM needs the recording's power-on fill first.
- Mapper banks aren't in the code count, nor in a routine's key: an address is the CPU's.
- `gp progress` doesn't read `gp agent bench` yet, so the effort lines game-printer printed beside the ladder aren't here.
