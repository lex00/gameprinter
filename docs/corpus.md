# The recordings corpus

This corpus is recordings: long runs of an original kept in the store, mostly imported movies ([movies.md](movies.md)), that reach a game's levels. It isn't the scoring corpus a game is scored against ([formats/scoring-corpus.md](formats/scoring-corpus.md)), though a run here can be harvested into mechanic clips (`gp mechanic harvest GAME RECORDING`) that end up in it. Recordings of commercial games are private: they live in `$GP_STORE` and nothing here publishes them.

Brought from game-printer's docs/corpus.md (its Milestone 1 corpus). The Super Mario Bros., Mega Man 2, Kirby's Adventure, Ninja Gaiden, Super Mario Bros. 2 and Gauntlet sections stay there, since those games aren't in gameprinter; the Donkey Kong section is here with its IDs and commands brought up to date. The recording IDs below are game-printer's store's; `gp mechanic import` doesn't copy store recordings, so they are in a store only if it was game-printer's.

## Donkey Kong (`donkey-kong-ju`)

These recordings are of Donkey Kong (JU), the ROM entry `donkey-kong-ju` of the games checkout's `donkey-kong/roms.toml`. The package scores `donkey-kong-oe` (Donkey Kong: Original Edition); JU is a second set the page plays ([mechanic-page.md](mechanic-page.md)), so these runs are history: nothing scores on them.

TASVideos was searched on 2026-11-02 and no file was downloaded. TASVideos' NES Donkey Kong is [game 210](https://tasvideos.org/210G). Its current publication, [1352M](https://tasvideos.org/1352M) by Phil, plays the game version "Donkey Kong (World) (Rev 1)", SHA-1 `d8dfacbfec34cdc871d73c901811551fe1706923` ([API](https://tasvideos.org/api/v1/games/210)). The publication runs `01:08.67` and was made with FCEUX `2.1.1`. That is the `ines_sha1` of `donkey-kong-ju` in roms.toml, so the movie is for our ROM: its `romChecksum` is MD5 `6d4a94c344463e562344249e18e9b99f`, the entry's `md5`. The NES version has three stages (25m, 75m, 100m; no 50m). The other listed version, "Donkey Kong (J) (PRG0)", isn't our dump.

The user approved the import on 2026-11-02. The movie file (philc2-donkeykong.fm2, 4,127 frames) is kept under `$GP_HOME/work/tas/donkey-kong-1352M/`, outside the repo, and was imported with `gp movie import --record --url https://tasvideos.org/1352M` as `33f7cf7e7989` (today the command also needs `--source donkey-kong-ju`). It desyncs: Mario never scores, loses his lives in 25m and the run ends at the title screen. `gp movie diagnose` finds no wasted input at shift 0. Shifts of -2 to -4 keep Mario climbing longer (y 172 at frame 800, 149 at 1,100) but he still dies in 25m, and `--ram-init fceux` changes nothing.

- 2026-11-02, steps 1 to 5 of [desync.md](desync.md): the header says `emuVersion 20100` (FCEUX 2.1.0, before the new PPU existed) and has no RAM init field. Select and Start land on the title as the movie means (game B is chosen by frame 8). Mesen 2.1.1 plays the same input log (game-printer's `gp oracle record`, `187d7524a660`) to the same places as `gp-nes`: Mario climbs 25m on the movie's route and reaches the top girder by frame 1,000 in both, then dies to a barrel between frames 1,350 and 1,400. Shifts -3 to +3 with zeros, ones and FCEUX's fill all lose lives the same way; the fill never matters, since the game doesn't read RAM it hasn't written.
- The cause is the random numbers. The main loop is `JSR $F4ED; JMP $C7E1`, and `$F4ED` steps an LFSR over `$18` to `$1F` on every pass while the game waits for the next NMI. How many passes fit in a frame depends on the emulator's CPU and PPU timing, so the barrels choose differently on FCEUX 2.1.0's old PPU than on `gp-nes` and Mesen, which agree with each other. No import option fixes that, and `gp-nes` isn't wrong (Mesen agrees), so the movie stays out of the corpus. A run made on an accurate emulator (BizHawk, or FCEUX with the new PPU) would be needed.
- 2026-11-02, the user approved downloading other runs. TASVideos lists two publications and 19 submissions for game 210 ([API](https://tasvideos.org/api/v1/Submissions?Games=210)). Four were imported, movie files kept under `$GP_HOME/work/tas/donkey-kong-<id>/`:

| Run | Made on | Result in `gp-nes` |
|---|---|---|
| [5254M](https://tasvideos.org/5254M), "all items" (publication, BizHawk 2.9, a conversion of an FCEUX 2.1.1 run) | BizHawk 2.9, NesHawk | desyncs: lives ($0055) fall from frame 700 and the run ends at the title. Shifts -3 to +3 with zeros or ones don't fix it. Mesen plays its log too (`5125e3e5bd23`) |
| [8038S](https://tasvideos.org/8038S) (cancelled) | BizHawk 2.8 | refused: made on another ROM, SHA-1 `7f6b63bfc5b3f54ff3e040be63e3e60df22fd3ca` |
| [4151S](https://tasvideos.org/4151S) (cancelled) | FCEUX 2.2.2 | desyncs: lives lost by frame 3,000 at every shift tried |
| [5658S](https://tasvideos.org/5658S), "100%" (cancelled, the author calls it a work in progress) | FCEUX 2.2.3 | syncs through 25m and 75m into 100m, with lives ($0055) holding at 2 from frame 411 (score 9,100 at frame 3,600, stage byte $0053 = 4). Mario dies once in 100m at frame 4,261 (lives 2 to 1); whether the run meant to isn't known. The run ends at frame 5,234 in 100m without clearing it |

5658S at shift 0 with zeros is recording `76c9bb11a914`, harvested in game-printer with `gp mechanic harvest donkey-kong-ju` in 5 s: 89 events over the three stages (25m 41, 75m 28, 100m 20), 15 example clips. The same today is `gp mechanic harvest donkey-kong 76c9bb11a914`. Its Mesen run is `fe3601b16af6`. It covers all three stages, so it serves as Donkey Kong's walkathon up to frame 4,261, with the caveat that it doesn't clear 100m.

- 2026-11-11, in gameprinter: 4151S imported and recorded with no wasted input, and game-printer and gameprinter gave the same recording ID ([recording.md](recording.md#movies)).
## Tapper

Tapper's runs are MAME `.inp` recordings, imported the same way ([movies.md](movies.md#arcade-movies)). On 2026-11-12 a 1,282-frame run made in MAME 0.276 imported, recorded and replayed frame-exact, and matched MAME's own capture on every frame ([recording.md](recording.md#arcade-recordings)). It was a test of the import; no Tapper run is listed here as a corpus run yet.

## Screens visited, per game

Each game can name the RAM bytes that say where the player is, in a definition named after its ROM entry ID under `coverage/` ([coverage.md](coverage.md)), which `gp coverage` reads to count levels and screens. Donkey Kong has no definition there yet; its stage byte is `$0053` (1, 3 and 4 in 25m, 75m and 100m of 5658S, and 2 for Original Edition's 50m), which its `observe.toml` `[level]` reads. Moss Hop's is `coverage/mosshop.toml`. game-printer's table for Super Mario Bros., Mega Man 2 and Kirby's Adventure, and how its sources disagreed, stays in its docs/corpus.md.

## Human play

Human play fills in what TAS runs skip. On the mechanic page ([mechanic-page.md](mechanic-page.md)) a person plays the original and records clips, kept as presses in the game's mechanic sessions. A run of a person's play goes into the store with `gp record --provenance human-play`, into a scratch store (`GP_HOME` set to a temporary folder) unless the person chose the real one.
