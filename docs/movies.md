# Importing movies

`gp movie import` turns an FCEUX `.fm2` or BizHawk `.bk2` movie, or a MAME `.inp` recording of an arcade set, into a Game Printer input log, which `gp record` records like any other run. Published tool-assisted runs, such as those on TASVideos, enter the store this way. The files stay outside the repo: `.gitignore` covers `*.fm2` and `*.bk2`, and the repo holds no third-party run.

Brought from game-printer's docs/movies.md. The import, its refusals and the hash rules are the same; the ROM hashes now come from each game's roms.toml instead of sources/nes.toml, `--source` has no default, a MAME `.inp` is new, and the tables of Super Mario Bros., Kirby's Adventure, Mega Man 2, Ninja Gaiden and Super Mario Bros. 2 movies are left in game-printer, since those games aren't here. [recording.md](recording.md) has the recording side, and [corpus.md](corpus.md) the Donkey Kong runs.

## Importing a run you downloaded

TASVideos publishes each run on a page such as `https://tasvideos.org/<id>M`, with a download link. TASVideos licenses its publications under CC BY 2.0 (see [Sharing](#sharing)), but the repo still holds no movie files, and recordings made from them are private like every recording of a commercial game:

1. Download the file yourself and keep it outside the repo, for example in `$GP_HOME/movies/` (default `~/.gameprinter/movies/`).
2. Import and record it in one step, with the publication page as its provenance:

   ```sh
   gp movie import ~/.gameprinter/movies/5658S/MOVIE.fm2 --source donkey-kong-ju --record --url https://tasvideos.org/5658S
   ```

   This looks up the ROM entry `donkey-kong-ju` (an ID in the games checkout's `donkey-kong/roms.toml`) in `$GP_ROM_DIR` by hash and records the run into `$GP_STORE` the way `gp record` does, with frame trace, events and checkpoints. The recording is named after the file, without its extension, unless `--name` says otherwise. Its provenance is `tas-import` with the URL, or with an empty URL if `--url` is left out. The index notes name the source file, its BLAKE3 hash, the emulator that made it and any shift. `gp recordings show <id>` prints them.
3. Check that it replays: `gp replay <id>`, or `gp replay --all`.

`--shift N` moves every input N frames later (or earlier, for a negative N), and `--ram-init fceux` records with FCEUX's power-on RAM fill; both are found per movie with [desync.md](desync.md).

Without `--record`, the command writes the input log to `--out FILE` (or stdout) and prints the `gp record` command for it. There is no default source: give `--source ID` for a ROM entry of a game's roms.toml, or `--rom FILE` for a ROM file. A file that no roms.toml names is still accepted with `--rom`, as "the given ROM (not in the registry)".

The store keeps the input log. Keep the downloaded file too if you want to import it again later, for example after a runtime change.

## What is imported

Only the controller buttons. Every frame of the log the import writes is marked polled. Recording replaces those flags with the frames the game read, so lag frames come from our runtime. Frame `n` of a `.bk2` becomes frame `n` of the input log, played from power-on. An `.fm2`'s first line comes before our frame 0, so its line `n + 1` becomes frame `n` (see below).

| Format | Read from | Buttons |
|---|---|---|
| FCEUX `.fm2` (version 3, text) | `key value` header lines, then one `\|commands\|port0\|port1\|port2\|` line per frame. A `length` header, when present, is the movie's frame count, and lines past it are ignored as FCEUX ignores them. It counts the first line, which is dropped (below), so the log has `length - 1` frames | Each port is 8 characters in the order `RLDUTSBA`; any character other than `.` or a space is a held button. `port1 1` makes a two-controller log |
| BizHawk `.bk2` (a zip) | `Header.txt` and `Input Log.txt` | The `LogKey` line names each column (`P1 Up`, `P2 A`, `Reset`, `Power`), so columns are read by name, whatever their order or letters |
| MAME `.inp` (format 3, made with `mame -record`) | A 64-byte header naming the system and the MAME that made it, then a zlib stream of one record per frame | Coin 1, start 1, tilt and player 1's stick and tap, into an arcade log (`gp-arcade-input 0`). Record N holds the ports the game reads during frame N+1, which is the log's entry N |

The import exits 2 on anything an input log can't express:

| Refused | Why |
|---|---|
| A start from a savestate or save RAM | Recordings start at power-on |
| PAL mode | Only NTSC is recorded |
| Four Score, more than two gamepads, a Zapper or another device | Input logs hold gamepads only, and `gp-nes` attaches two |
| Famicom Disk System | Not supported by `gp-nes` |
| Binary `.fm2` input | Rare; save it as text in FCEUX |
| A power cycle after the first frame (FCEUX command `2`, BizHawk `Power`), a BizHawk `Reset` on the first frame, or an FCEUX disk or coin command | An input log has no way to say it |
| An `.fm2` without a `port0` line | Controller 1's device is unknown |
| An `.fm2` whose first line holds buttons, or a disk or coin command | The first line comes before our frame 0 and is dropped, so its input would be lost |
| A `.inp` that presses player 2, coin 2 or service, or has the DIP switches off their defaults | An arcade log holds player 1's controls only |
| More than 2,000,000 movie frames (over nine hours, counting an `.fm2`'s first line), or a `.bk2` part over 64 MiB | A bound on what a crafted file can make the import allocate; no published NES run is that long |

A power cycle on the first frame (FCEUX command `2`, BizHawk `Power`) is accepted, since every recording starts at power-on. A soft reset later in a movie (FCEUX command `1`, BizHawk `Reset` after the first frame) is kept: the input log presses reset at the start of a frame (input log format 1, [formats/recording.md](formats/recording.md#input-log)), and `gp-nes` presses it there. FCEUX acts on a line's command after that line's frame, so an `.fm2` reset on line `k` is pressed before the frame made from line `k + 1`, and one on the last line is refused. When a BizHawk `Reset` lands isn't checked against a real `.bk2` yet; it's pressed before its own frame.

An `.fm2`'s first line is always dropped, whatever its reset or power command (none, `1`, `2` or both). FCEUX's first movie frame comes before our frame 0, so the movie's second line is our frame 0. A first line that holds buttons is refused, since dropping it would lose input. game-printer checked the rule on 2026-10-27 against seven published Super Mario Bros. runs: every one that syncs does so with the first line dropped, and three that start with no command lose a life early with it kept. Its docs/movies.md has the table.

## Which ROM a run was made on

Each NES movie names its ROM by hash, and the import accepts it only if that hash is the target source's. Recordings of one revision say nothing about another, so a run made on another revision is refused with exit 2 and a message naming the ROM entry it was made on. A hash that no roms.toml has is refused too; the message gives the hash and the ROM file name from the header.

| Emulator | Header field | What it hashes | roms.toml field |
|---|---|---|---|
| FCEUX | `romChecksum base64:...` (or `0x` and hex) | MD5 of PRG and CHR, without the header or trainer (FCEUX `src/ines.cpp`) | `md5` |
| BizHawk | `SHA1` (or `MD5`) | The hash BizHawk's game database gives the ROM. For a ROM not in its database, the SHA-1 of the whole file including the iNES header (`Database.GetGameInfo` in BizHawk's `Database.cs`) | `ines_sha1` (whole file with a plain iNES 1.0 header) or `sha1` (PRG and CHR); `MD5` against `md5` |
| MAME | The header's system name | The set by MAME's name, not a hash | The romset entry's `system` |

Each `[[source]]` of a game's roms.toml holds these hashes (Donkey Kong's two entries have all three). `gp movie hashes ROM` prints them for a ROM file and checks them against the roms.toml entry with the same SHA-256; a ROM entry with none of them can't take a movie, and the import says to add them from `gp movie hashes`. A whole-file hash depends on the header bytes, so `ines_sha1` uses a plain header: bytes 0 to 6 as in the file with byte 6's trainer bit cleared, byte 7's high nibble (the mapper's) kept, and bytes 8 to 15 zero, followed by PRG and CHR without any trainer. A `.bk2` made from a file with other header bytes names a hash roms.toml doesn't have, and the import refuses it, printing that hash. With `--rom FILE`, that file's own whole-file SHA-1 is accepted too.

## Arcade movies

A MAME `.inp` names its set by MAME's name (`tapper`, `rbtapper`), so the target must be that romset: `--source` a romset entry of Tapper's roms.toml, or `--rom` the set's zip. An `.inp` against an NES ROM, or an NES movie against a romset, is refused in words. The log it writes is an arcade log, and `--record` records it on gp-arcade. `--ram-init` is unread there: the board powers on one way. [recording.md](recording.md#arcade-recordings) has the commands and the run checked against MAME on 2026-11-12.

`gp movie diagnose` reads lag marks, which an arcade log doesn't keep, so on an arcade recording it finds nothing.

## Sync

A run syncs when our runtime, given its buttons, reaches the same states its emulator did. Expect many to fail at first. Emulators differ in power-on state and input polling and timing. FCEUX and BizHawk also count frames from their own boundaries, which may not line up with ours (a frame ends at the start of vertical blank, [formats/recording.md](formats/recording.md#frame-boundary)). A run that doesn't sync still records; it just goes somewhere else.

Not here yet: game-printer's Mesen oracle (`gp oracle record`, `gp oracle diff`), which showed where a run leaves the intended path on another accurate emulator. The MAME oracle ([oracle-mame.md](oracle-mame.md)) does that for an arcade set.

## When a movie desyncs

[desync.md](desync.md) gives the procedure, step by step. `gp movie diagnose <recording>` lists wasted inputs. game-printer's `scripts/movie/` (`ram_drift.py`, `sweep.sh`, `variants.py`, `fcm2fm2.py`) isn't in gameprinter; desync.md says what each step does without them.

## Tests

`just check` runs the unit tests in `crates/gp-cli/src/movie.rs` and `crates/gp-cli/src/diagnose.rs`: synthetic `.fm2`, `.bk2` and `.inp` files give the input logs they should, the first-line rule and `--shift` hold, every refusal above exits with its reason, a `.bk2` may name its ROM by any of its hash forms, and an `.inp` names its set or is refused. The diagnose tests check that a press released before the next read is wasted and one held into it isn't.

Not here yet: game-printer's `just check-movie-roms`, which recorded synthetic Super Mario Bros. movies against a committed baseline.

## Sharing

TASVideos' [Links and Redistribution](https://tasvideos.org/WelcomeToTASVideos#LinksAndRedistribution) section, read 2026-10-27, says: "TASVideos submissions and publications are licensed under the Creative Commons Attribution 2.0 license (CC BY 2.0)." If you are not the author, you may "Freely redistribute our TASes in any medium or format" and "Remix, transform, and build upon our TASes for any purpose, even commercially", and in return must "Properly and accurately attribute all authors of the TAS", "Link to the correct publication, submission, or Userfile on TASVideos", "Properly label everything as tool-assisted" and not modify publication encodes. It also asks people not to link directly to input files. The site license page ([SiteLicense](https://tasvideos.org/SiteLicense)) says "All content on the site is under the Creative Commons Attribution 2.0 license."

So a movie's inputs, and the input log imported from it, may be shared on those terms. The recording made from it may not be shared. Its trace and checkpoints hold the game's RAM and PPU state, which stay private like every recording of a commercial game. The recording's provenance keeps the publication URL; the authors are on that page and in the movie's header.
