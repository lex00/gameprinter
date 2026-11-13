# Donkey Kong 25m: a retrospective of the first assisted game

Brought from game-printer's docs/research/ as written on 2026-11-12. Paths are game-printer's at the time: templates/donkey-kong-ju/ is the package games/donkey-kong in the games checkout (scored on donkey-kong-oe now), ~/.game-printer is ~/.gameprinter, sources/nes.toml is each package's roms.toml, and loop.toml is gone.

Written 2026-11-02 for the user's request: "walk through this process from start to finish. We have lots of stuff here that is scripted but implied. If someone started Donkey Kong again from scratch, still using human assist just like this time, what can we make smoother? What can help for the future?"

NES Donkey Kong (JU) went from a registered ROM to a 25m model in one day (2026-11-02, about 09:47 to 20:41 local time, UTC-7). At c610add3 the gate's status file has 59 of the 60 clips of corpus `donkey-kong-ju-25m` 2.0.0 at 100% of frames together straight through, with the original's random draws. The 60th, clear-25m 001, is exact from its start to frame 3446 and first splits at 3447, the first frame 75m is drawn; docs/benchmarks.md puts that frame out of scope, so its 25m part is exact too. Picture is scored apart and is far lower (phase 8).

This note is a snapshot. It describes what happened, sorts each step into scripted, implied or by hand, lists what went wrong, and ranks what to build or write so the next game goes faster.

## Sources

| Source | What was taken |
|---|---|
| `git log` on m3/stage0-coverage from 6f752800 to c610add3 | 127 commits, 64 of them named "Donkey Kong"; times are commit times, UTC-7 |
| templates/donkey-kong-ju/ | model.toml (2,044 lines, 78 clip citations), mapping.toml, probes.toml (41 probes), loop.toml, fidelity.toml, clips.toml, corpus.toml, ext/ (2,337 lines of Rust in three files) |
| `gp mechanic queue donkey-kong-ju` and `~/.game-printer/mechanics/donkey-kong-ju/*/talk.ndjson` | The quest log's 19 rows and the person-agent turns (no sealed session, no clear-25m clip 002, no walk clip 006 opened) |
| `~/.game-printer/work/refine/donkey-kong-ju/status.json` (version 22) | Per-clip scores at c610add3 and the last five merges |
| `~/.game-printer/gate/runs/` | 165 run manifests of the gate's home: 118 `gp corpus score` runs (80 whole corpus, 38 `--only`) |
| The session's scratch folder | dk-switches.md, dk-purist-checklist.md, dk-cycle-bytes.md, dkB-barrel-notes.txt, about 40 dk*.sh and dk*.py scripts, the gate logs, pipe/tools/dkclips.py and dkset.py |
| The coordinator's transcript and the workers' transcripts | The user's messages with their times, the workers' reports and corrections, load averages |

## Who

| Who | Role | Active (local) |
|---|---|---|
| The user | Recorded about 27 clips handed off in the talk logs, 3 sealed full runs, and made the day's rulings (random graded by behaviour, stretches, purist switches, one pipeline for every game) | 09:39 to 20:41 |
| Coordinator | The main session: briefs, relays, rulings written down, sealing by hand. Ran out of context once (about 13:20) | all day |
| Worker A | Clips, probes, the planned 25m clear, the hammer, ladders, the ladder-zip | 09:47 to 20:40 |
| Worker B | Hammer locks, the first barrel notes | 11:38 to 12:34 |
| Worker C | Asset map, then the only model editor until the gate, then gated merges | 11:38 to 20:43 |
| Workers D, E, G | D: step cycle, fires, smash. E: climb, falls, deaths (ran out of context, ended 17:24). G: barrels v1 to v7, the RNG count (ran out of context, ended 19:32). F was a one-shot research fork | 12:19 to 20:40 |
| UI worker | The mechanic page: hand-off, row filing, compare, seal, refinement tab | 14:11 to 20:19 |
| Pipeline, scripting, two presentation, sound and cycle workers | Corpus and gate; `gp mechanic probe`; the picture; the sound driver and `gp cycles` | 17:07 to 20:23 |

The machine has 18 cores. Load averages from 60 to 106 were logged during the afternoon, and the page lagged until background jobs were reniced to 15.

## Timeline by phase

| Phase | When (local) | Took | Main commits |
|---|---|---|---|
| 1. ROM in, template skeleton | 00:12 (registered), 09:47 to 09:49 | 2 min of work once started | 31917622, 6f752800 |
| 2. Player, level, TAS | 09:52 to 11:48 | about 2 h | b5a6aa6e, 775b6a2a, 0f9bab17, f38851f7, 67dbde23 |
| 3. First mechanics | 09:52 to 11:07, then probes all day | about 1 h 15 min for walk, jump, ladder, hammer, deaths | 3c23f833 |
| 4. Clips from the human | 09:52 to 20:38, in six bursts | about 27 hand-offs | e3551174, 89697c25, b7b604c0 |
| 5. Modelling | 11:41 (goal set) to 20:34 | about 9 h | 3c23f833 to c610add3 |
| 6. Scoring | 12:33 (first table) to 17:34 (in gp) | 5 h on private scripts | 4bf599e8, d5c835fd, e44e87f5, fc1fdc76 |
| 7. Merge gate | 18:00 (corpus) to 20:34 | first gated landing 19:12 | 3b8ae9cf, 94d0cf1e, 4d1ffe17, efb9a752, 9d7a23ba, 723cc973 |
| 8. Presentation | 17:20 to 19:38 | about 2 h 20 min | 3347cd40, 6647bb2c, 790a9f0c |
| 9. Sound | 17:56 to 20:22 | about 2 h 30 min | df3459db, ef3be03d, 5b59640e, c414c203 |
| 10. Cycle timing | 12:02 (counts) to 18:48 (`gp cycles chain`) | measured, not modelled | 2022aeb2, e3296d3c |
| 11. Sealed held-out runs | 13:54 (first run) to 20:43 (scored) | three runs, scored by hand | b7b604c0, 94d0cf1e |

The clips at 100% went 43 of 61 meeting the bars (18:00, corpus 1.0.0), 45 of 60 (18:59, corpus 2.0.0), 49 (19:10, first gate), 56, 57, then 59 of 60 at 20:34. Of the 127 commits, 25 landed after 19:10 through the gate.

## 1. ROM in and the template skeleton

The ROM was registered in sources/nes.toml at 00:12 with Q*bert and Track & Field. When Donkey Kong became the focus (09:39), worker A wrote the skeleton in two minutes: template.toml, a model header (platformer 0.4, a placeholder system order) and an asset map holding only the iNES header's CHR ROM as tiles. `gp model check` and `gp template check` passed and `gp mechanic start donkey-kong-ju walk` served the page.

| Step | Kind | How |
|---|---|---|
| Copy and identify the ROM | Scripted | `gp roms add`, `gp roms scan` |
| Register its hash | By hand | An edit to sources/nes.toml |
| Template, model header, asset map stub | Implied | Worker A knew the shape from SMB and Moss Hop; no command makes them |
| loop.toml, fidelity.toml, observe.toml, probes.toml | Implied, and late | Written at 18:00 and 18:13, eight hours in |
| Quest log rows | By hand | Ten rows typed with `gp mechanic queue --status next` |

What was slow: nothing here took long, but the files that the standard pipeline needs (corpus, contract, loop recipe, gate defaults) were not made at the start, and that cost most of phase 6.

## 2. Finding the player and the level

Blind discovery failed at play start: its rules assumed a player faster than Mario's 2 px every 3 frames, and his x lives only in the sprite buffer. A fix for slow walkers (a364f28f) and a blind farm from the user's path (10:27 to 11:00) kept Mario on the bottom girder; the user stopped it at 11:01. Worker A read Mario's x ($46) and y ($47) off the walk and jump clips and wrote them into mapping.toml (10:13).

TASVideos 1352M named our ROM's hash but desynced in 25m. Worker A found why in six minutes: the game steps its random register on every pass of the wait-for-frame loop, so a movie made on FCEUX 2.1's old PPU loses its barrels (0f9bab17). 5254M and 4151S desynced too, 8038S was for another ROM, and 5658S synced through 25m and 75m into 100m (10:26). `gp mechanic harvest` cut it into 89 events per stage once the stage byte $53 was named in observe.toml.

Worker C traced the stage drawing routine's reads and found 25m's layout as a PPU update string at $F55B (67dbde23, 11:48).

| Step | Kind | How |
|---|---|---|
| Player bytes | By hand | Read off RAM dumps of the walk clip; blind discovery found none |
| Stage byte | By hand | $53 typed into observe.toml |
| TAS search | By hand | Four submissions tried one at a time; `gp movie import` defaults to `--source smb1-ntsc-prg0` |
| TAS import, desync check, harvest | Scripted | `gp movie import`, `gp movie diagnose`, `gp mechanic harvest` |
| Level layout | By hand | A trace of the draw routine's reads, then an asset map entry |

What was slow: the player and stage bytes are the two facts every later tool needs, and both were found by hand. The TAS search was manual.

## 3. First mechanics

From 09:52 to 11:07 the user recorded walk, jump, ladder, hammer and death clips, and worker A answered each with measurements in the page. Each answer came from a one-off script that joined the session's path to the clip's log, recorded it and dumped RAM a frame at a time (dkclip.sh, dkprobe.py, then dkh*, dkhp*, dkb-* logs and contact sheets).

Three answers were wrong and corrected in later turns: the walk speed (1 px every 2 frames, then 2 px every 3), the hammer (measured on a stretch that started at Game Over, then on the right frames), and the walk-off fall (reported as survived, then as a death). At 10:03 the user said "you are asking me for clips they would resolve", meaning the jump sweep and walkathon could make what the agent asked for.

At 18:13 the scripting worker turned these measurements into `gp mechanic probe` and templates/donkey-kong-ju/probes.toml: 41 probes, 404 checks, 22 agent and player clips reproduced input for input. Its inventory found most 25m measurements were by hand or half scripted before that.

| Step | Kind | How |
|---|---|---|
| Measure a clip | By hand until 18:13, then scripted | Scratch scripts, then `gp mechanic probe` |
| Turn to the person | Scripted | `gp mechanic say --show --measure` |
| Clip a probe as evidence | Scripted | `gp mechanic clip-save`, `gp mechanic video` |
| Ask for a clip only when no sweep can make it | Implied | Memory notes, not a check in any command |

## 4. Clips from the human

The user handed off clips in six bursts: 09:52 to 10:00 (walk, jump), 10:44 to 11:07 (ladders, hammer, deaths), 13:54 (a full run), 16:35 to 16:44 (four rows added after the user asked "do you have the right clips? do you need help?"), 18:02 to 18:27 (two sealed runs, a hammer fall) and 20:38 (title, two players, game B).

| Problem | When | Fixed by |
|---|---|---|
| The page felt stuck and laggy; A pressed again after Enter did nothing visible | 10:00, 14:10 | e3551174 (a hand-off answers at once); renice of background jobs |
| A clip was filed under the row open when it arrived, not the row it was recorded for | 16:45 ("the row should be recorded immediately when i hit enter/a") | 89697c25 |
| A full run went under walk (walk 006) because there was no seal action | 13:54 | b7b604c0 added Select to seal, at 18:56 |
| A fireball clip was filed under double-barrel-jump clip 3 | 16:44 | Noted in the quest log by hand |
| Moving a sealed clip out of clear-25m by hand freed its number, and the next hand-off reused clip 2 | 18:23 to 18:26 | Not fixed |
| Side by side and diff views seemed gone | 18:02 | They only ever appeared with `say --against`; b7b604c0 made compare reachable for any clip |
| WATCH REFINEMENT said "not refined yet" everywhere | 18:29 | Status is written only by the gate, from 18:56 |

The rows the user recorded for in the afternoon were added only when the user asked. The quest log's notes and statuses are typed by agents with `gp mechanic queue --note`: at 20:34 it still says hammer "clips score 0 to 8%" and climb-ladder "agent working", while the status file has both at 100%.

## 5. Modelling

The user set the goal at 11:41: model all of 25m, 90% or better on every clip. The work split by area (A: Mario and the hammer; B then G: barrels; D: step cycles and fires; E: climbing, falls and deaths; C: level, score and timers). From 12:34 to the gate's arrival, C was the only editor of the template: the others sent patches to a scratch folder and C merged them in order, rescoring every clip after each.

The branch gained 68 named features in feature_list.rs and mapping_features.rs over the day, most of them for Donkey Kong (power.locks, run.step_cycle, jump.takeoff_fraction, jump.land_hold, climb.up_cycle, climb.down_through, jump.fatal_drop, input.keep_direction, action.add, field.bcd, slots.resync, field.lookup and others). Barrels, Kong's throws, the fires, the hammer and the sound driver went into ext hooks (2,337 lines). Eleven engine quirks became profile switches in the `normal` preset (profiles.toml, dk-switches.md), after the user's ruling that every behaviour a purist would expect is modelled behind a switch.

Many rules were read by hand from the ROM with a scratch disassembler: the fires ($E1E5-$E833), the sound driver ($FA48-$FE58), $43's reuse between stages, the barrel's odd-y fall step, the points sprite, the ladder-zip. Each has a row in deterministic-inference.md naming the tool that would make it unnecessary.

| Step | Kind | How |
|---|---|---|
| Which routine wrote a wrong byte | Scripted | `gp element inputs` |
| Reading a routine | By hand | No disassembly is pinned for DK; a scratch disassembler and ROM reads at offsets (dkzip.py) |
| Splitting work by measure | Implied | dkfirst.py held a table from measure prefix to worker letter |
| Merging a patch | By hand until 19:10 | C applied patches, then dkgate.sh and dkA-rebase.sh rebased them |
| Checking a claim before reporting it | Implied, and missed twice | See phase 6 |

What went wrong:

- One editor was the bottleneck. From 12:34 until the gate took patches at 19:10, every template change went through C, who rescored all clips after each merge; a duplicate barrels worker was started and stopped at 12:34.
- Workers E and G ran out of context while holding open work, and the coordinator ran out once. Their open items were handed over by message.
- Workers committed through the main checkout's shared git index in the morning (10:17 to 10:49): one worker's staged copy of mechanic.rs would have undone another's commit, and another worker waited for that to clear before committing.
- One worker's `rm -rf` removed another worker's worktree, and about 200 GB of per-worker build folders filled the disk (memory note "workers-no-rm").

## 6. Scoring

From 12:33 to 17:34 every score was a private script around `gp interp compare`. Each worker had its own copy: dkrun.sh, dkrunA.sh, dkrunA2.sh, dkrunBT.sh, dkrunBTb.sh, dkrunBTx.sh, dkrunC.sh, dkrunG.sh, with dkscore.py, dkresync.py, dkstretch.py, dksplits.py, dkfirst.py and dktable*.py on top. Each joins the session's path log up to the clip's start with the clip's log, runs `gp interp draws` and `gp interp compare --from START`, then counts frames together, resync splits and stretches. The ROM path is hard-coded in each.

| Problem | When | What happened |
|---|---|---|
| Shared output folder | found 13:43 | dkrun.sh wrote into one scratchpad/dkl/; a barrels table read another worker's result. C switched to dklC/; dkrunA.sh still read dkl/ in its resync branch |
| Draws only by script | 14:19 to 17:34 | Random draws scripts existed (4bf599e8) but scoring took them only through each worker's script |
| Resync dropped draws | to 17:34 | Each load restarted the draws script, so a resync run with `--draws` desynced at its first split (fixed in d5c835fd) |
| Sites in another order | to 16:04 | Two draws on one frame in another order desynced the script (e44e87f5) |
| Draws cache key | 19:30 | The cached script was keyed by clip alone, so after the fires' new draw site walk 001 looked like a regression; a patch was blamed for the scorer (fc1fdc76) |
| False claims from frame pairing | 16:54 to 17:02 | A blamed Mario's step counter for a split at 14127, pairing the wrong frames (model n is original n+3). C then claimed 29 lag frames in the pause, reading "no input" lines as unpolled frames, and retracted it |
| The SMB pipeline unused | to 17:07 | The user asked "are we using the frame refinement code now that existed for smb?"; DK had no corpus, contract, `gp solve` or `gp picture` until then |

The stretch rules came from the user during this phase (13:17 the longest exact stretch, 15:21 the low end) and went into docs/benchmarks.md and then into gp (d5c835fd).

## 7. The merge gate

The pipeline worker built the corpus with a one-off import (pipe/tools/dkclips.py and dkset.py): a hard-coded session list and a skip list, writing clips.toml and the presses files. Version 1.0.0 (18:00) had 61 clips, one of them walk 006, the user's first sealed run filed under walk. Version 2.0.0 (18:27) dropped it and wrote the rule into the clip set's header.

`gp model gate` arrived at 18:11 and needed no setup from 18:47. The first whole-corpus score in the gate's home took 327 s; touched-clip scores took a median of 4 s after that, with the tip's scores cached.

| Problem | When | What happened |
|---|---|---|
| The tip moved | 19:35 on | Patches against an older tip failed to apply (model.toml at line 1712). dkgate.sh and dkA-rebase.sh did a 3-way apply, kept both sides of added-only conflicts, rebuilt gp and gated again, up to four tries |
| Asset map not followed | to 19:18 | The gate's run kept its first asset lift, so the sound worker's `sound_rom` slot failed until someone lifted by hand (9d7a23ba) |
| A clean run scored 0 | 19:57 to 20:16 | Once clear-25m was exact up to the clear scene, the stretch scorer dropped that stretch for ending outside play, so the longest stretch read 0 and every patch failed the bar (723cc973) |
| Exit codes | all evening | 107 of the 118 `gp corpus score` runs in the gate's home exited 1. The command exits 1 when a clip's curve on the original differs from the corpus's, is measured again or can't be made, or the mapping's schema moved; the gate reads the score record and not the exit code (gate.rs, `score`), so the manifests can't tell a broken score from a routine one |

From the first gated landing (19:12) to c610add3 (20:34), 25 commits reached the branch, most of them through the gate, and no clip at 100% was lost.

## 8. Presentation

`gp picture` first ran on Donkey Kong in the pipeline worker's runs (17:07 to 19:10): fidelity 16 of 100, behaviour 91, picture 0. Every body was a fallback box and the backdrop palette differed on every frame. The first presentation worker drew from the game's own data (6647bb2c, 19:32): picture 31, 58% of background frames the same. The second added the points sprites (790a9f0c) and two effect phase features (c8f44c25).

| Step | Kind | How |
|---|---|---|
| Picture score | Scripted | `gp picture` |
| Background queue | Scripted, with a bug | `gp capture background --steps 60` scored 8 clips and stopped; a second call scored the other 52. Two workers hit it; cause not found |
| Draw order, flicker, sprite limit | Not started | Can't be told apart until the model has real sprites |

What was slow: picture was first measured about five hours after the first model commit (12:05), so the behaviour work went on with no pixel check.

## 9. Sound

The sound driver's data went into the asset map (df3459db), and the driver itself was read by hand from a scratch disassembly and copied as the dk_sound hook (633 lines). It keeps 27 bytes; stepped from the original's bytes at each NMI it agrees on 7206 of 7206 frames of the user's play and 5223 of 5223 of run 5658S, checked with `gp cycles chain --ram-out` and a scratch checker. Later fixes put $5D in the model and moved the death sound to the right frame.

| Step | Kind | How |
|---|---|---|
| Find the driver | By hand | The routine that writes $4000-$4017, found by reading |
| Check a copied step | Half | `gp cycles chain --ram-out` gives the RAM; the comparison was a scratch script |

## 10. Cycle timing

The RNG count per frame (67 to 369 steps) was measured at 12:02. Worker G showed that the model's state explains 78% of its variance and the exact count on 3% of frames, and proposed not modelling it; the user ruled at 17:02 that every behaviour a purist would expect is modelled behind a switch. `gp cycles` (17:54) and `gp cycles chain` (18:48) then reproduced the count exactly on 6000 frames of the user's play and 5000 of 5658S from the frame's RAM. The frame's code reads 279 bytes in 25m. dk-cycle-bytes.md sorted them by hand into groups by their effect on the handler's cycles. The rng_per_frame switch itself was not started; scoring uses the original's draws meanwhile.

## 11. Sealed held-out runs

The user recorded three full 25m runs to hold out. All three were handed off before the page could seal: the first went under walk (walk 006), the second under clear-25m with an agent turn saying it was sealed, the third under clear-25m as clip 2 before the coordinator moved it with `mv` into `$GP_HOME/sealed/` (a loop script, sealwatch.sh, did the same for a sealed-run row). Worker C's full rescore had already scored walk 006, and the pipeline worker's `gp solve` report on corpus 1.0.0 named its first split; both workers reported that they took nothing from it. The final sealed scoring at 20:43 was a hand-typed command run from a copy folder.

| Step | Kind | How |
|---|---|---|
| Seal at hand-off | Scripted from 18:56 | Select in review (b7b604c0) |
| Keep sealed runs out of every scorer | Half | The page's refinement view filters them; clips.toml's header states the rule; the import script had a skip list; `gp corpus score` and `gp solve` don't check |
| Score the sealed runs | By hand | A coordinator command, no gp verb |

## What was scripted, implied and by hand

| Step | Today | Gap |
|---|---|---|
| ROM in | `gp roms add`, `scan` | Registering the hash is an edit |
| Template skeleton | Nothing | No `gp game new` |
| Player and stage bytes | Blind discovery, which failed on a slow walker | Read by hand |
| TAS | `gp movie import`, `diagnose`, `gp mechanic harvest` | Finding a movie that syncs is by hand |
| Measuring a clip | `gp mechanic probe`, `contacts`, `video` | Probes came at 18:13 |
| A clip as a power-on input log | Nothing | Every worker wrote it (dkrun*.sh, dkclip.sh, dkprobe.py, bevy_clips.py) |
| Sessions to corpus | `gp corpus init` once clips.toml exists | clips.toml was written by a one-off script |
| Scoring one clip | `gp interp compare`, `gp corpus score --only` | Before the corpus, private scripts |
| Draws, resync, stretches | In gp since 17:34 | Scripts before |
| Merging | `gp model gate` | Rebasing onto a moved tip is a script |
| Quest log | `gp mechanic queue` | Typed by hand, never fed by scores |
| Who owns a split | dkfirst.py's table | Status.json has `owner` null |
| Sealing | The page from 18:56 | Scoring them is by hand |
| Reading a routine | `gp element inputs` for writers | A listing is a scratch disassembler |

## Recommendations, ranked by time saved on the next game

The savings are estimates from the timeline above, not measurements. "General" means it helps any game; "DK" means it mostly helps Donkey Kong or games built like it.

| Rank | What to build or write | What it does | Grounded in | Scope |
|---|---|---|---|---|
| 1 | `gp corpus import GAME` | Reads every mechanic session, writes clips.toml and the presses files and runs `gp corpus init` or `bump`. Leaves out sealed* sessions, any clip whose start state or inputs match a file under `$GP_HOME/sealed/GAME`, rows marked out of scope, and title demos by a flag. The page runs it after each hand-off, so the corpus exists from the first clip | The corpus came eight hours after the first clip, from dkclips.py with a hand list; walk 006 got into 1.0.0 | General |
| 2 | `gp game new GAME --rom ROM --genre platformer` and docs/runbook-new-game.md | Writes template.toml, the model header, an asset map from the header, a mapping stub, observe.toml, fidelity.toml with `[corpus]` bars, loop.toml with `[gate]` on the current branch, an empty probes.toml, the quest log seeded from the genre's checklist (docs/research/genres.md), and makes the gate's run. The runbook lists every step in this note's tables as a checklist, in order, with the command for each | Phases 1 and 6: the standard pipeline was set up at 17:07 to 18:47 after the user asked; five hours of private scoring before it | General |
| 3 | `gp clip log GAME SESSION:N` and `gp clip score GAME SESSION:N [--model DIR]` | The first writes a clip's input log from power-on (path to its start, then its presses), checked against the start's state hash. The second scores it as `gp corpus score` would (draws, resync, stretches, first lasting split) into `$GP_HOME/work/score/$GP_WORKER/`, never a shared folder | Eight dkrun variants, the shared dkl/ overwrite, hard-coded ROM paths | General |
| 4 | `gp model gate --rebase` (or `--branch B`) | When the patch doesn't apply on the moved tip, 3-way merge it there, keeping both sides of conflicts that only add TOML tables or array entries, and fail with the conflict only when a hunk changes the same lines. Takes a branch or commit as well as a patch file. Gate from the first model commit, not from the eighth hour | dkgate.sh and dkA-rebase.sh; C as sole editor from 12:34 to 19:10 | General |
| 5 | Quest log fed by the gate | `gp model gate` updates each row's status and note from status.json (done when every clip of the row meets the bars; the clip and frame of its first lasting split otherwise). `--note` stays for what a person reads, but scores are never typed. `owner` comes from a `[owners]` table of measure prefixes in loop.toml, replacing dkfirst.py's table | Rows at 20:34 still quoting 0 to 8% for clips at 100%; refinement tab empty until 18:56 | General |
| 6 | Sealed runs as a step of their own | A "sealed run" row exists in every new game's quest log. `gp corpus score`, `gp solve`, `gp picture` and `gp interp compare --clip` refuse any input matching a sealed file. `gp sealed score GAME` scores the sealed runs on the tip for the coordinator only and appends one line (model hash, percentages, longest stretch) to a ledger under `$GP_HOME/sealed/GAME/`. The page asks "seal this?" when a clip on a mechanic row runs longer than a stage (the stage byte changes) | All three sealed runs misfiled; walk 006 scored by two workers; sealed scoring by hand | General |
| 7 | Frame pairing in every compare | `gp interp compare` and `gp clip score` print the pairing at the top ("model frame n is original n+3; 0 lag frames skipped") and every split as both frames. A worker brief line: a claim about a frame quotes both frame numbers from the tool | Two false claims and a retraction between 16:54 and 17:02 | General |
| 8 | A worker harness | `gp worker env NAME` prints the exports a worker needs: `GP_WORKER`, a private out folder, the shared `CARGO_TARGET_DIR` for the branch, `nice 15`, and a `--jobs` cap from the load average. Briefs start from a template in docs/agents.md that says no rm, one task, end with a report and an open-items list | Load averages to 98 on 18 cores; a worktree deleted by another worker's `rm -rf`; 200 GB of build folders; E, G and the coordinator out of context | General |
| 9 | A scope end in the mapping | `[scope] while = "..."` (for DK, the stage byte $53 equal to 1). Scoring stops at the first frame out of scope and reports the rest apart, as docs/benchmarks.md now asks, with no frame found by hand | Frame 3447 of clear-25m 001 was found by hand | General |
| 10 | Random draw sites found by the tool | `gp interp draws --find`: run the clips, find the bytes a shift register steps every frame (A7, Berlekamp-Massey on the byte's history), list each instruction that reads them as a candidate `[[draws]]` site with its mask, and set `score_draws`. Then draws are in scoring from the first clip | The $DB7B `AND $19` site was traced by hand; draws reached scoring at 17:34 | General |
| 11 | `gp movie find GAME` | Lists the TASVideos submissions whose ROM hash matches the template's source and runs `gp movie diagnose` on each, printing which syncs and how far. `gp movie import` takes the source from the ROM's hash instead of defaulting to smb1-ntsc-prg0 | Four movies tried by hand | General |
| 12 | Player and stage bytes from the human's first clips | `gp mapping propose GAME --from walk,jump` runs the player finder and the stage-key finder (deterministic-inference.md, 2026-11-01 and 2026-11-02 rows) on the person's clips instead of blind runs, and writes the mapping's first rows for review | Blind discovery failed on a slow walker; $46/$47 and $53 were typed by hand | General |
| 13 | `gp picture` in the first score | `gp corpus score` runs `gp picture` on the same clips (or `gp game new` sets it in loop.toml), and the `gp capture background --steps` early stop is fixed | Picture first measured in the evening; one background call scored 8 of 60 | General |
| 14 | `gp code listing ROM [--from PC]` | A labelled listing of a routine under `$GP_HOME/work`, labels from the mapping and the asset map, cycles per instruction from gp-cycles. Never committed. Removes the scratch disassembler | Fires, sound, $43, ladder-zip and points read with a scratch disassembler | General |
| 15 | Detectors for the hand-read cases | The slot-loop object tracer and the sound-driver finder already named in deterministic-inference.md (2026-11-02 rows), and the event-edge detector from the dk-pipeline row | Phase 5's hand reads | General |
| 16 | Hooks reviewed for features | Each ext hook gets a list of the rules in it that another game could use (a ladder choice by a masked random byte, a crowd limit, a slot loop of rolling objects), as candidates for genre features | 2,337 lines of DK-only Rust | DK first |
| 17 | Exit codes that say what failed | `gp corpus score` exits 0 when it scored, 1 when a clip is below a bar, 2 when the original's curves differ from the corpus's, 3 when it couldn't run; the gate's manifests then show real failures | 107 of 118 scoring runs exited 1 in the gate's home | General |
| 18 | The idle-loop RNG switch | Build rng_per_frame from `gp cycles chain`'s cost function once the 279 bytes are model state | Phase 10 | DK, and any game that steps its RNG in the idle loop |

### The mechanic page

The human-assist moments that cost the user time, and what would make each smoother:

| Moment | What happened | What to change |
|---|---|---|
| The first clips | The agent asked for clips a sweep could make (10:03) | `gp mechanic say` refuses a clip request while a sweep for that mechanic exists and hasn't run (`gp mechanic sweep KIND --list`); the page shows "the sweep is making this" instead |
| Hand-off | Up to 10 s with no answer; a second A press | Fixed (e3551174). Keep the sending state's counter visible over the game, as now |
| Which row a clip goes to | Filed by the row open on arrival | Fixed (89697c25). Also refuse a clip on a done row without a note, and show the row name before recording starts |
| A full run | Filed under walk | A RECORD FULL RUN button that seals by default, so the user never has to pick a row for a held-out run |
| Clip numbers | A number reused after a sealed clip was moved out | Number clips by a counter kept in the session, never by the files present |
| What to record next | The user asked "do you have the right clips?" at 16:30 | Rows that are waiting on the person show first and say why, from `gp mechanic leads` and the rows' open items, without being asked |
| Watching progress | The refinement tab empty until the gate wrote status | `gp game new` makes the gate's run, so the tab has the tip's scores from the first model commit |
| Compare views | Overlay, side by side and diff hidden unless `--against` | Fixed (b7b604c0) |
| Rulings | Six rulings in one day reached the docs by agents' commits | A RULING button that records the user's words with the time into the session, so a worker can cite it |

## What is DK-specific

The RNG stepped in the idle loop, $43 reused between stages, the ladder-zip, the barrel's 4-barrel crowd limit and the sound driver copied as a hook are Donkey Kong's. Recommendation 18 and most of 16 serve them. Everything ranked 1 to 15 and 17 is a step the next game repeats whatever its genre: the sessions become a corpus, the corpus is scored with draws and stretches, patches pass the gate, and sealed runs stay sealed.
