# Sources: getting, checking and registering source games

The process for bringing a source game into Game Printer, from a file you have to a known source that recordings, clips and packages can name by hash. Every step has a `gp` command so people and agents do it the same way. It came from game-printer's docs/sources.md. The registry there was one file, `sources/nes.toml`; here each game's ROMs are listed in its own folder in the games checkout, in `roms.toml` and in `package.toml`'s `[[rom.entry]]` list. [Bring in a game](../site/src/content/docs/bring.md) walks through the same steps for a new game.

## Rules

- ROMs never go in a repo, even ones you own. They live in `$GP_ROM_DIR` (default `~/.gameprinter/roms`). `gp` refuses to use a ROM folder inside a git work tree, including through a symlink. The pre-commit hook refuses ROM files by their contents and by their hashes, and refuses any binary file that isn't allowlisted, which catches headerless dumps.
- ROM data typed out as numbers is refused too. See [ROM bytes in text](#rom-bytes-in-text).
- Only use copies of games you own. A dump from your own cartridge is the cleanest source.
- A game's folder holds facts about its sources (hashes, revisions, headers), never their data. The one exception is a short excerpt, a few bytes, quoted to document how revisions differ.
- Everything downstream names a source by its hash, never by file name. File names from ROM sets are labels, and labels can be wrong.

## 1. Get a copy

The preferred route is to dump your own cartridge with a USB dumper (for example INLretro or Retrode). Save the result as an iNES (`.nes`) file. An arcade game is a MAME romset, a zip named for its MAME system ([below](#arcade-romsets)).

## 2. Add it

```sh
gp roms add ~/Downloads/game.nes            # copies into $GP_ROM_DIR/nes/
gp roms add --move ~/Downloads/game.nes     # or moves it there
gp roms add ~/Downloads/rbtapper.zip        # a romset, into $GP_ROM_DIR/arcade/
```

`roms add` checks that each file is a valid NES image or a romset it knows, writes it and reads it back to confirm the copy. It then prints its identity (the headerless SHA-256) and the game whose `roms.toml` names it. A ROM no game names yet is added too, as unknown: that's the one `gp game new` starts a game from. It refuses to overwrite a different file with the same name.

A dump with junk around the game data can go in cleaned instead (see [Junk around a dump](#junk-around-a-dump)):

```sh
gp roms add --clean ~/Downloads/game.nes    # adds a cleaned copy and its sidecar; the original stays
```

### ROMs game-printer registered

`roms/known.toml` in the tool repo holds the six NES entries game-printer's `sources/nes.toml` registered for games that have no folder here yet: Kirby's Adventure (`kirby-usa-prg0`), Ninja Gaiden, Gauntlet, Gauntlet II, Q*bert and Track & Field, with their hashes, board facts where a cartridge database confirmed them, evidence and notes. Nothing names a ROM by them, so `roms scan` still calls these ROMs unknown and `gp game new` can start a game from one; when it does, it copies the matching entry into the new game's `roms.toml` under the new game's source id, instead of drafting one from the header. gp-nes's compiled-in list, `roms/nes.toml`, stays empty: a game's board facts live with the game.

## 3. Identify it

```sh
gp roms scan                    # every NES image and romset in $GP_ROM_DIR, by contents
gp roms scan --json             # with full hashes and header fields
```

The identity is the SHA-256 of the PRG and CHR data with the header removed, so a re-headered or renamed copy of the same dump gets the same identity. It's the hash No-Intro uses. `roms scan` names each file by the game whose `roms.toml` holds its hash, read from the games checkout (`--games`, else `$GP_GAMES_DIR`, else a `games` folder at or beside the current folder or one above it). It also reports junk around the game data, one line per finding, and whether a file that matches no source may be a modified copy of one.

## 4. Compare revisions

When a game has more than one revision or region, find out exactly what differs before choosing a target:

```sh
gp roms diff a.nes b.nes --summary
```

`roms diff` compares PRG and CHR separately and names each differing range by the first image's asset map and labels: graphics, level data, a table, text, code or data. `--summary` leaves out the bytes, so its output can be quoted. Either side may be a raw PRG binary, such as a disassembly build. Two images whose programs sit at different file offsets are compared as the CPU sees them with `--cpu C000-FFFF`. [gp roms diff](roms-diff.md) has the rest.

## 5. Verify it

A source is `unverified` until its hash is confirmed by one of these references, each independent of the file we were given. The first is stronger.

| Reference | Check | Record in `evidence` |
|---|---|---|
| A disassembly build | A community disassembly assembles to a PRG image identical to the source's PRG, checked with `gp roms diff source.nes build.bin` | The disassembly, its version and the assembler |
| Published checksums | The source's hashes match checksums quoted by at least two independent public sources: cartridge databases such as NesCartDB (BootGod's, which BizHawk ships as `Assets/gamedb/NesCarts.xml`), ROM set lists such as GoodNES as TASVideos quotes them, or disassembly repositories | Each source, the hash it gives, and that this is weaker than a rebuild |

The entry's `status` says only `verified`; whether that came from a rebuild or from published checksums is in its `evidence`.

A disassembly covers program data only, unless the game has CHR RAM and keeps its graphics in PRG. Graphics data in CHR ROM is covered by published whole-ROM checksums, which hash PRG and CHR together. There is no No-Intro DAT check.

Checksums come in several forms. NesCartDB gives the SHA-1 and CRC32 of PRG and CHR without a header, which is the entry's `sha1`. GoodNES lists give the SHA-1 of the whole file with GoodNES's header, which is `ines_sha1` when GoodNES's header is a plain one. A disassembly repository may name the SHA-1 of the file it was made from.

## 6. Register it

`gp game new GAME --rom ROM --genre G` writes the entry for a ROM no game names yet, into the new game's `roms.toml`, with the hashes and the mapper, marked `unverified`, and the matching `[[rom.entry]]` in its `package.toml`. `--source ID` names the entry otherwise (`qbert-u`), for a game that will have more than one ROM. A second ROM of a game you already have is a `[[source]]` entry you add to its `roms.toml` by hand, with an entry in `package.toml`. `gp package scan` and the gate say where the two disagree.

| Field | Meaning |
|---|---|
| `id` | Stable ID used by recordings, clips and packages, `<game>-<region>-<revision>`, such as `donkey-kong-ju` |
| `title`, `platform`, `region`, `revision` | Facts about the release. `platform` is `nes` or `arcade` |
| `system` | An arcade source's MAME set name, such as `rbtapper` |
| `sha256` | The headerless SHA-256 from `gp roms scan`, or an arcade set's identity |
| `mapper` | The NES mapper number, from the board. An arcade source has none |
| `board`, `submapper`, `mirroring`, `battery`, `prg_rom_bytes`, `chr_rom_bytes`, `prg_ram_bytes`, `chr_ram_bytes` | Board facts: the board's name, and what a header would otherwise say. `mirroring` is `horizontal`, `vertical`, `four-screen` or `mapper` (the mapper chip sets it). `prg_ram_bytes` counts battery-backed RAM too; `battery` says whether there is a battery. Give all of them or none; an entry with some is refused. `gp roms clean` writes a header from them ([Junk around a dump](#junk-around-a-dump)) |
| `board_evidence` | Where the board facts come from, normally NesCartDB |
| `md5`, `sha1`, `ines_sha1` | Optional. The hashes FCEUX and BizHawk movies name their ROM by, from `gp movie hashes` ([movies](movies.md)). Without them, `gp movie import` can't accept a movie for this source |
| `status` | `unverified` or `verified` |
| `evidence` | How it was verified, or where the hash came from |
| `notes` | Anything that matters later, such as how it differs from other revisions |
| `kind`, `[source.patch]` | For a patched source ([below](#patched-sources)) |

The package's `[[rom.entry]]` gives the same `id`, `sha256` and `platform`, the entry's `role` (`scored` for the ROM the corpus is scored on, `play` for one the package only loads to play), its asset map and that map's pins.

Not here yet: `gp sources list`, the registry printed, and `gp assets find`, which found a known game's assets in a new dump of it by content and printed a draft entry. For a new dump of a game you have, compare it with the one you have (`gp roms diff --summary`, with `--cpu` when its program sits elsewhere) and write its entry and asset map from what the diff says.

## Arcade romsets

An arcade game comes in as a romset: a MAME zip named for its MAME system, such as `rbtapper.zip`. Game Printer knows a set by its chips, never by the file names inside: each chip's SHA-1 must match the set's list from MAME's own source. The set's identity, the `sha256` its `roms.toml` and `package.toml` give, is the SHA-256 of its chips' SHA-1s, sorted, one a line, so it is the same for any zip that holds those chips. The sets known so far are Tapper's: `rbtapper`, `tapper`, `tappera`, `tapperg` and `sutapper`. `gp roms add` checks the zip against the sets and copies it whole to `$GP_ROM_DIR/arcade/`. `gp roms clean`, `gp roms diff` and `--patch` are for NES images.

## Patched sources

A ROM hack is a base source plus a patch its author hands out, usually an IPS or BPS file. Game Printer never holds either: the person downloads the patch from the author's page, `gp roms add --patch` applies it to their own base ROM, and the game's `roms.toml` holds only hashes and URLs.

```sh
gp roms add --patch ~/Downloads/hack.ips                       # every registered base in $GP_ROM_DIR
gp roms add --patch ~/Downloads/hack.ips --base BASE-ID
gp roms add --patch ~/Downloads/new-hack.ips --base BASE-ID --new   # prints a draft entry, writes nothing
```

1. The patch is found among the games' `roms.toml` entries by its SHA-256. An unknown patch is refused, since IPS has no checksum of its own and nothing else could check the result. `--new` applies it anyway and prints a draft entry with the result's hashes, to check and paste.
2. Each base the patch is registered for (or the one `--base` names, by ID or by 8 or more hex digits of its hash) is found in `$GP_ROM_DIR` by hash, and cleaned by [the header rules](#the-header-rules) first, so a junk header can't move the patch's offsets.
3. The patch is applied in memory, in Rust (`crates/gp-evidence/src/patch.rs`). BPS checks its own source, target and patch CRC32s.
4. Every hash the entry holds for the result (`sha256`, `md5`, `sha1`, `ines_sha1`) must match, or nothing is written.
5. The result goes to `$GP_ROM_DIR/nes/patched/<id>.nes`, through a temporary file read back and checked, with a sidecar `<id>.patch.toml` holding every hash. The patch itself is kept as `$GP_ROM_DIR/patches/<sha256>.<format>`, so the result can be made again.

A patched source is a `[[source]]` entry with `kind = "patched"` and a `[source.patch]` table: the base it is made from, the format, its size and offsets, the URL and page it comes from, its author, contact, the `permission` they gave and the credits. Its own hashes and board facts are the patched image's, so `gp roms scan` and `gp record` find it by hash like any dump.

The pre-commit hook refuses patch files by name (`*.ips`, `*.bps`, `*.ups`, `*.xdelta`) and by their magic bytes, and `.gitignore` lists them. Once a patched source is registered and its result is in `$GP_ROM_DIR`, the hook's [ROM bytes check](#rom-bytes-in-text) covers the hack's bytes too.

A hack is read with its base's labels file (`gp labels`), since it keeps the base's code and RAM except where `gp roms diff --summary` says.

## Status

| ID | Game | Status | Notes |
|---|---|---|---|
| `donkey-kong-oe` | Donkey Kong: Original Edition (3DS Virtual Console) | unverified | The scored ROM of `games/donkey-kong`. CNROM: its upper PRG bank starts with Donkey Kong (JU)'s whole program, and the lower bank is new |
| `donkey-kong-ju` | Donkey Kong (JU) | unverified | A ROM Donkey Kong loads only to play ([templates](templates.md#roms-to-play)) |
| `tapper`, `rbtapper`, `tappera`, `tapperg`, `sutapper` | Tapper | unverified | MAME romsets, every chip matched by SHA-1 against MAME 0.276's lists. `games/tapper` |

No published checksum or cartridge database has been checked for any of them yet. Each entry's `evidence` says how its hash was measured.

## Clips from a source

A game's corpus plan (`corpus.toml`, [scoring corpus format](formats/scoring-corpus.md)) names its ROM by the SHA-256 its `roms.toml` gives, and each clip's start by presses from power-on. It holds no snapshot and no curve. An emulator snapshot holds RAM, nametables and palettes the game's code wrote from ROM data, so it stays under `$GP_HOME` (`work/snapshots/starts/`), and `gp corpus score` and `gp game score` derive the curves from the ROM each time.

## ROM bytes in text

The pre-commit hook catches game images and binary dumps by their contents and by hash. A table copied out of a ROM into source, a doc or a test is text, so the hook also runs `scripts/romguard/rom_text.py` on the staged diff.

What it guards is read from the repo, hashes only: `.githooks/guarded-roms.toml` (NES images by headerless SHA-256, and arcade chips by SHA-1 with their set, region and offset), `roms/*.toml`, every `*/roms.toml` and every `package.toml`'s `[[rom.entry]]`. Every source game-printer's `sources/nes.toml` listed is in `guarded-roms.toml`, so a game it registered stays guarded before its package comes over.

It reads the added lines of each staged text file, finds runs of byte values written as numbers, and refuses a run that holds 16 or more consecutive bytes of a guarded ROM or arcade chip present in `$GP_ROM_DIR`. The ROMs are found by hash, the same way `gp roms scan` finds them, and a set's chips that load one after another are also read joined, so a table typed across two chips is found. When no guarded ROM is present this part does nothing and says so, so a clone without ROMs can still commit. A staged file whose hash is a guarded image's or chip's is refused whole, ROM or no ROM.

| Written as | Example |
|---|---|
| Hex with a prefix or suffix | `0x12`, `$12`, `0A6h`, `\x12` |
| Bare hex, with separators | `12 34 ab`, `12,34` |
| A hex string | `1234abcd...`, of even length, 4 digits or more |
| Binary | `0b0001_0010`, `%00010010` |
| Decimal 0 to 255, with separators | `18, 52, 171` |

Only separators may sit between two tokens of a run: whitespace, the characters `,;:|-"'`, brackets, and the assembler byte directives `.byte`, `.byt`, `.db`, `db` and `hex`. A JSON list of quoted numbers is one run. So is a Markdown table row, or an assembler table with one directive per line. Each run is read once as written and once with comments blanked, so a table with a comment on each row is still one run. Bare numbers are tried as hex and as decimal. Windows too regular to say where they came from are skipped: fewer than 4 distinct values, a pattern of 4 bytes or fewer repeated, or a ramp with steps of one size, such as the APU triangle sequence.

A refusal looks like this, with the matched bytes' SHA-256 at the end:

```
pre-commit: refusing crates/x/src/lib.rs:3 (16 bytes of donkey-kong-ju at ROM offset 0x1000, written as numbers; sha256 ...)
```

The fix is to lift the data through the game's asset map ([Asset lifting](assets.md)) and read the lifted files from `$GP_HOME/work` at build or run time. Tests use made-up tables.

Excerpts under 16 bytes are never refused. A longer excerpt the [rules](#rules) allow (32 bytes at most) can go in `.githooks/rom-excerpt-allowlist`, one `<path> <sha256>` line per excerpt. A comment above the line says what the bytes document and who reviewed them. The entry holds for that file and those bytes only.

The 16-byte windows are indexed once into `$GP_HOME/work/rom-guard/`, in a file named by the ROMs' hashes, and reused on every later commit until the set of ROMs changes. `scripts/romguard/test_rom_text.py` and `scripts/test-hook.sh` test it with made-up ROMs, in `just check`.

## Junk around a dump

Dumps often carry junk around otherwise exact game data: text a ROM tool wrote into the header, a trainer, or bytes appended after the game. Game Printer finds each case by rules, reports it, and can write a cleaned copy. Cleaning only ever touches bytes outside the PRG and CHR data. A dump whose game data was changed (a hack, a translation, an intro patched into PRG) can't be cleaned, and nothing is ever guessed.

| Case | How it's detected | What's cleaned | What's refused |
|---|---|---|---|
| Junk header bytes | The header rules below: an archaic iNES header trusts only bytes 0 to 6 and reads bytes 7 to 15 as zero. Known stamps are named | Bytes 7 to 15 set to zero, or the header rebuilt from the entry's board facts | Nothing; the mapper comes from byte 6 |
| Trainer | Byte 6 bit 2 set and the file has room for 512 bytes after the header and the stated PRG and CHR | The 512 bytes, which aren't hashed either | A flag without room for a trainer is reported and ignored |
| Overdump | The file is longer than the header, the stated sizes and any trainer | The trailing bytes, which aren't hashed | Nothing |
| Underdump | The file is shorter than the stated sizes | Nothing | The file: it can't be identified or run, and is never padded |
| A header that contradicts the entry | The file's hash is a known source's, and its header (read by the rules) disagrees with the entry's board facts | The header, rebuilt from the entry, when cleaning is asked for | Nothing else |
| Modified game data | The cleaned hash matches no game's entry | Nothing | Nothing; it's reported as matching no known release |

### The header rules

The rules are the "Recommended detection procedure" on the NESdev wiki's iNES page (https://www.nesdev.org/wiki/INES).

- NES 2.0 if byte 7 AND `$0C` is `$08` and the sizes it then gives (with byte 9) fit in the file.
- Plain iNES if byte 7 AND `$0C` is `$00` and bytes 12 to 15 are all zero.
- Otherwise archaic iNES. The wiki says "iNES 0.7 or archaic iNES" here, and that an emulator "should either mask off the upper 4 bits of the mapper number or simply refuse to load the ROM". Game Printer masks: only bytes 0 to 6 are trusted, so the mapper comes from byte 6 alone and bytes 7 to 15 read as zero.

The known stamps are the ones FCEUX recognises and clears in `iNES_HEADER::cleanup` (src/ines.h), which looks for "DiskDude" or "demiforce" at byte 7 and "Ni03" at byte 10. Reports name them as `DiskDude!`, `demiforce` and `Ni0330`. Any other nonzero bytes in 7 to 15 of an archaic header are ignored the same way, just not named. "DiskDude!" read literally adds 64 to the mapper number: a file with byte 6 `$21` and byte 7 `D` (`$44`) would read as mapper 66 where the game is mapper 2 (UNROM).

### What `gp roms scan` says

```
  5314ad0c40616119  mapper 2   PRG  128K CHR    0K  unknown                nes/Some Game (U).nes
      header: bytes 7-15 hold "DiskDude!" (the DiskDude! stamp), ignored; mapper 2 from byte 6 (read literally, byte 7 would make it mapper 66)
      release: matches no known release
```

The lines are `header:` (ignored bytes, or a contradiction with the entry), `trainer:`, `overdump:` (trailing bytes), `underdump:` (as a problem: the file isn't identified), `release:` for a file that matches no source, and `file name tags` when the name carries GoodNES or No-Intro tags such as `[o]` (overdump), `[h]` (hack), `[p]` (pirate), `[T+Eng]` (translation), `[t]` (trained), `[b]` (bad dump) or `(Hack)`. Tags are hints only, never evidence: names can be wrong. When the mapper and the PRG and CHR sizes match a known source's, the `release:` line says so and that the file may be a modified copy of it. There's no way to say how many bytes differ, since the original's bytes aren't held anywhere. `gp roms scan --json` gives each finding with offsets and BLAKE3 hashes.

### Cleaning

```sh
gp roms clean ~/Downloads/game.nes -o ~/clean/game.nes
```

`roms clean` writes the header, PRG and CHR, and drops the trainer and anything after the stated sizes. For a source whose entry has board facts the header comes from them. For any other file it's the file's own header, with the bytes the rules don't trust set to zero and the trainer flag cleared.

Safeguards:

- The input is never changed.
- An output inside a git work tree is refused, as is one that would replace the input.
- An existing output is replaced only with `--force`.
- The copy goes to a temporary file first and is read back. Only a copy with the input's identity and no junk left takes the output's name.

Next to the copy goes a sidecar, `<out>.clean.toml`:

| Field | Holds |
|---|---|
| `input`, `input_bytes`, `input_sha256` | The file cleaned, its size and whole-file SHA-256 |
| `output`, `output_bytes`, `sha256` | The copy, its size, and the headerless SHA-256 the two share |
| `source`, `header_rule`, `header` | The known source, if any; `registry` or `file`; the new header |
| `contradictions`, `findings` | Where the header disagreed with the entry, and the scan lines |
| `removed` | Each removed run (`header`, `trainer` or `trailing`) with its offset, length and BLAKE3 hash. The original header is quoted in full; game data never is |

It exits 0 when the cleaned copy is a known source, 1 when it matches no game's `roms.toml` (the copy is still written, with the file's header), and 2 when it couldn't run. `gp roms add --clean` runs the same code and puts only the cleaned copy and its sidecar into `$GP_ROM_DIR`, leaving the original where it is; it exits 1 when a cleaned file matches no game's entry.

### Running a dump

gp-nes loads every ROM through `gp_nes::board::runnable`, which applies the same rules: junk is dropped before `tetanes-core` sees the file. Its own list of board facts is compiled in from `roms/nes.toml` in the tool repo, so a build boots a given ROM one way whatever games checkout `gp` is pointed at. That list is empty for now (Moss Hop and the test ROMs have clean headers), so every game's ROM runs with its own header, read by the rules. A dump whose header is wrong for its board needs `gp roms clean` first.
