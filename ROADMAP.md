# Roadmap

As of 2026-11-12. This replaces game-printer's ROADMAP.md, whose last version froze game and tool work on 2026-11-08 for the migration research. The research was adopted that day ([docs/evaluation.md](docs/evaluation.md), section 7), and the work since has been the migration into this repo and `lex00/games`, then the port of what game-printer had. This page says what is done and what is left. It gives no dates.

The full lists behind it are the port ledger and its three audits, kept outside the repo under `~/.gameprinter/notes/audit/` (`PORT-LEDGER.md`, `libraries.md`, `commands-pages.md`, `around.md`).

## Done

### The migration

| Step | State |
|---|---|
| The tool repo | gameprinter, a Rust workspace of 42 crates, with Moss Hop as its test game and `just check` needing no ROM |
| The games repo | `lex00/games`, one package folder a game: Donkey Kong (its OE and JU ROMs, one package, now 1.0.9) and Tapper |
| A home of its own | `~/.gameprinter`; game-printer's `~/.game-printer` is gone |
| Registry | `lex00/gp-registry`, a public git repo read at its raw GitHub address, unsigned by the owner's choice |
| The public page | [nes-donkey-kong.intentius.workers.dev](https://nes-donkey-kong.intentius.workers.dev): Donkey Kong from a ROM the visitor loads, with the clay theme `lex00/turnips` beside it |
| Game modules | A game's hooks in one Wasm module, built from a kit in a sandbox with no network, the same bytes on a Mac and on Linux, run in the page's Worker and in gp under wasmtime with a deadline and a memory cap ([docs/modules.md](docs/modules.md)) |
| Gates | `gp model gate` for games and `gp tool gate` for the tool, with batches, three-way merges and append-only rows; the tool gate publishes each landing's build ([docs/gate.md](docs/gate.md)) |
| Workers | `gp worker env`, the locked cargo and the rules in [docs/workers.md](docs/workers.md) |

Where the migration went another way than its plan is in [docs/migration-differences.md](docs/migration-differences.md): a git registry instead of R2, no official root key, signing off, `gp loop` left out, and the old repo kept as a read-only bundle.

### The port

The ledger gives everything game-printer had that wasn't here yet a wave.

| Wave | What | State |
|---|---|---|
| 0 | Safety: the `assets lift` symlink guard, the ROM hook guarding arcade romsets and every hash in the ROM registry, the games gate's lost behaviour, the page checks wired back | Landed |
| 1, 1b | Corpus import, init and bump; sealed runs; model mechanics; labels; roms diff; the lost flags on commands both tools had | Landed |
| 2 | Probe, moments, slots, translate, odds, scenarios, picture, solve | Landed |
| R | `gp model refine --lockstep`, `gp rule`, `gp model questions`, part of gp-extract | Landed |
| M | The mechanic page, now `gp serve`'s tabs CLIPS, REFINE, PLAY and THEMES ([docs/mechanic-page.md](docs/mechanic-page.md)) | Landed |
| 3a+, 3b | gp-events, gp-explore, gp-capture; record, replay, query, events, trace, movie, recordings, watch, explore, capture; code listing and tables, draws, element inputs, cycles, reach, model static | Landed |
| 4 | gp-track, gp-coverage, gp-gaps, gp-sprites, gp-imagefmt and their commands; sound capture, compare and fidelity | Landed |
| 5 | gp-extract, gp-derive, gp-validate; model derive, validate, blind and propose; dataset rom; mechanic harvest, leads and sweep | Landed |
| 6 | The arcade counterparts on gp-arcade: recording, instruction traces, an access log with the PC, the Z80 disassembler and code map, the sprite list and tilemap, AY logs, MAME `.inp` import | Landed |
| 7 | The farms, the store commands, gp-host, gp-mcp, the capture-run loop, `gp author` | Landed |
| E | The editor (`editor/`) and the rest of `gp serve` | Landed |

Dropped on record: the Bevy generator and export, `gp loop`, other games' code (SMB, SMB2, MM2, SMB Remastered), Rivet Rodeo and its paper theme. Rivet Rodeo, the open version of Donkey Kong, was deleted by the owner on 2026-11-11; a JU ROM now plays JU's own game.

## Left

### Wave 8: old features folded into what replaced them

- The inspector, as parts of `gp serve` and the `show explain` and `request` commands.
- The agent clip loop.
- The old theme pipeline.

### Wave 9: the rest of the porting side

Each comes after what it depends on.

| Area | Items |
|---|---|
| Games and levels | Level kit, chunks, catalog, progress, scorecard, findings, localize |
| The NES oracle and accuracy | The Mesen oracle (gp-oracle), the NES test ROMs |
| Model and assets | `model diff`, `mutants`; `assets find`, `hash`, `stub`, `open`; `template extract`, `pack`, `cites`, `derive`, `font` |
| Sprites and sound | `sprites schema`; `sound render`, `decode`, `drivers`, `banks`; gp-sound's driver registry. `sprites export` and `import` and `sound export` and `import` came over and were removed on 2026-11-13 |
| Video and clips | `gp video`, `mechanic video`, `compare`, `contacts`, `clip-save` |
| Corpus and datasets | `corpus show`, `diff`, `verify`, `reduce`; `input-log`; interp baselines; `dataset grade`, `gaps`. `dataset export` and `catalogue` came over and were removed on 2026-11-13 |
| Measuring | `gp bench`, metrics and runs, gp-metrics, gp-chunks, gp-wasm-check replay, gp-assets pointer hashing, per-hook affected-clip hashing |
| Small ones | `docs`, `env`, `noop`; the server's status and capture progress routes |

### Wave 10: around the code

Docs and tests land with each wave. The wave-10 branch brings the three agent skills (`.claude/skills/`, with the commands that aren't here yet named as such), the checklist for bringing in a game (the site's bring page, "The steps, in order"), `schema/` with `gp self schemas` and `gp model schema`, the ROM-tier tests (`profiles_dk` in part, `clip_agree`, `roms clean` and `roms scan` end to end, the junk-dump boot) under `just check-roms`, the recipes `fmt`, `test`, `site`, `site-dev` and `docs`, the pre-merge hook, the site's prose lint, and the workflow docs whose commands have landed. Left after it:

- Six source entries: Kirby, Ninja Gaiden, Gauntlet, Gauntlet II, Q*bert, Track & Field.
- Donkey Kong's `evidence/` score records, including the whole-corpus baseline the evaluation put at step 3.3.
- `profiles_dk`'s frame-by-frame hammer and jump checks, which need a module run's counters on every frame.
- The wasmtime, x86_64, Docker and tetanes vendor checks.
- The recipes whose scripts didn't come: `upgrades`, `build-timings`, `bench`.

### Wave T: tooling gaps found by modelling Tapper

Opened 2026-11-12. The owner paused model work on Tapper (games branch `tapper-2`) until the tools can do what was done by hand.

Already in the tree: the Z80 code map behind `gp code` on a romset, the access log behind `gp element inputs` and `gp reach`, `gp labels propose` from that log, `gp interp draws --find` on the main Z80 with draws recorded in the arcade lockstep, `gp model refine --lockstep` passing a field no mechanic owns and tuning hook numbers (`ext_params`), `gp package scan`'s allowance for press numbers, `gp game test`, and the games repo's hook preferring the published gp.

Still open:

- `gp model static --clips` and `--unread` on the Z80. The rest of `gp model static` reads a romset's main Z80 today.
- `gp model blind` on an arcade board. The faithful port on branch `arcade-tools` finds play start and the bartender's x, then stops looking for a jump. Seven changes to the method (ties between objects that move together, lanes, held actions, moves from what discovery found, progress from a rising counter, the drawn check, system order from the access log) wait for the owner's answer before any is made.
- `gp interp moments` and `gp rule` on arcade, and a rule for lanes: `gp rule` has only `jump`.
- Components for a player that switches lanes.
- `gp track` and the probe checked on the board's sprite list.

### Smaller open items

- A signed module kit inside each tool release (T3). The kit is made from a checkout today.
- Frame timing of a module on phones (P4). Only an Apple M5 Pro in headless Chrome has been measured.
- The registry can't take submissions from other people, and the page reads one registry.
- On the site's own "Not built yet" lists: a theme preview before publishing, a theme kit with its checks, level packages a player can pick, playing a game made here on a page others can open, and sending a change from a fetched folder without a fork.

## Decided

- 2026-11-08: the research defines the direction. GitHub repos under lex00 (private at the time) with a self-hosted runner on the owner's Mac for ROM jobs. The tip was imported as it was, with its two template failures as known failures. `clips.toml` comes across unchanged until phase 4.4. gp-gen-bevy's shared drawing code came in as `gp-look`. Donkey Kong stays one package; Moss Hop stays only as the tool's test fixture.
- 2026-11-09: signing off for the registry while there is one publisher. No official root key.
- 2026-11-11: Rivet Rodeo and the old page retired; `lex00/gp-registry` replaces `lex00/rivetrodeo`.
- 2026-11-12: the tools before the game. Tapper's model waits for wave T.
- `gp model blind` is ported faithfully though it "never worked that well", so it can be improved, possibly on Tapper.
