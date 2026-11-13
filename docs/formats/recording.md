# Recording format, version 1

A recording is everything the pipeline knows about one run of an original game: what went in (inputs), what happened (traces and events), and how to get back to any frame (checkpoints). Every part is an object in the evidence store, named by its BLAKE3 hash, and the manifest names them all.

The manifest's JSON Schema is `schema/recording.json`, generated from `RecordingManifest` by `gp self schemas`.

Brought from game-printer's docs/formats/recording.md. The format is unchanged and gameprinter reads game-printer's recordings: same manifest, input log, frame hashes, canonical state (field table version 5), trace, event stream and checkpoints. What changed around it: the store's default place, no SQLite index (the recordings log is read each time), and the arcade romset source and its arcade input log. The measurements on Super Mario Bros. are game-printer's. [recording.md](../recording.md) has the commands.

## Where recordings live

In the evidence store at `$GP_STORE` (default `$GP_HOME/store`, so `~/.gameprinter/store`), which `gp` refuses to place inside a git work tree. Objects are at `objects/<2 hex>/<62 hex>`. The manifest is itself an object, and its hash is the recording's ID. The store's `index/` directory holds the recordings log (see Index below). Recordings of commercial games are private and are never published or committed.

## Manifest

JSON, defined by `RecordingManifest` in `crates/gp-evidence/src/manifest.rs`. The manifest describes what was run and nothing else. Two runs that agree on every field below have the same manifest and so the same ID. Who recorded them and when make no difference. Everything else about a recording is in the index.

| Field | Meaning |
|---|---|
| `schema` | Recording format version (1) |
| `source` | The game. For NES: `{"kind": "nes-rom", "sha256", "mapper"}`, where `sha256` is the headerless PRG and CHR hash. The title comes from the ROM entry in a game's roms.toml with that hash. For an arcade romset: `{"kind": "romset", "identity", "system"}`, the set's identity (the SHA-256 of its chips' SHA-1s, sorted) and MAME's name for it (`tapper`); added 2026-11-12 within schema 1, so older builds refuse it as an unknown kind. For a game model run by the interpreter: `{"kind": "model", "name", "bundle"}`, where `name` is the model's `model.name` and `bundle` the hash of a store object holding the model's canonical text and the lifted asset files it reads, as JSON. The `model` kind was added on 2026-10-28 within schema 1. A model recording's `power_on` holds the defaults, which nothing reads. Since 2026-10-28 it has a `trace` (state `GPSP`) and `events` (schema 2), described under [model traces and events](#model-traces-and-events); one made before has neither. Its runtime is `gp-interp`, whose version also names the `ext` hooks it ran, with a hash of their crate. gameprinter reads model recordings; `gp record --model`, which made them, isn't here yet |
| `runtime` | `name`, `version` and, for runtimes we build, the source `commit`. Mesen recordings (`gp oracle record`) say `mesen2`, `2.1.1` and its commit. An arcade recording's is `gp-arcade`, its version with the board's and the recording's. Replay treats a recording made by a different runtime as stale ([recording.md](../recording.md)). For `gp-nes`, `version` is its behaviour version, the `tetanes-core` release and a hash of our patch to it, such as `2 (tetanes-core 0.17.0 + patch ede4ce9d0dcb)` where the patch hash is the first 12 hex digits of the patch file's BLAKE3 hash, and `commit` is the upstream `tetanes-core` commit (third_party/README.md) |
| `power_on` | Everything that changes how the console behaves from power-on (next table) |
| `frames` | Frame count; equals the input log's frame count |
| `input_log` | Hash of the input log |
| `checkpoint_interval` | Frames between checkpoints |
| `frame_hashes` | Optional: hash of the frame hash object (below). Replay needs it |
| `checkpoints` | `[{frame, snapshot}]`, strictly increasing, none past `frames` |
| `trace` | Optional: the frame trace (below), `{schema, chunk_frames, fields, chunks}`: the chunk schema (1), frames per chunk, the hash of the field table object, and the chunk hashes in order. The chunks cover frames 0 to `frames` exactly |
| `events` | Optional: the event stream (below), `{schema, chunk_frames, chunks}`, chunked the same way |
| `provenance` | Where the input came from: `human-play`, `tas-import` with `url`, `bot` with the bot's `name`, or `scripted`. A scripted run has no name here, since the input log's hash already identifies the script |

`RecordingManifest::validate` enforces the rules in the last column.

Both `trace` and `events` were drafts with other shapes until 2026-10-27 (`trace` had no `fields`, and `events` was a list of hashes). Nothing wrote either before then, so no stored manifest has the old shapes, and the change didn't need a new schema.

Every `power_on` field is always present. A runtime powers on from these values alone and refuses any it can't reproduce. An unknown field is an error, because it could change behaviour. `gp record` uses the defaults below.

| Field | Default | Meaning | `gp-nes` |
|---|---|---|---|
| `region` | `ntsc` | `ntsc` or `pal` | Both |
| `ram_init` | `zeros` | `{"kind": "zeros"}`, `{"kind": "ones"}`, `{"kind": "fceux"}`, or `{"kind": "random", "seed"}` | `fceux` is FCEUX's default fill of internal RAM, `00 00 00 00 FF FF FF FF` repeated, with other RAM as zeros (added 2026-10-27 within schema 1; older builds refuse it as an unknown kind). `gp-nes` writes it after loading the ROM, and game-printer's Mesen capture wrote it before frame 0. Refuses `random`: `tetanes-core` can't seed it |
| `cpu_ppu_offset` | 1 | Master clock cycles the PPU runs behind the CPU ([CPU-PPU clock alignment](https://www.nesdev.org/wiki/PPU_frame_timing#CPU-PPU_Clock_Alignment)) | Fixed at 1 in `tetanes-core`; refuses others |
| `ppu_warmup` | false | Whether the PPU ignores some register writes for about a frame after power-on ([PPU power up state](https://www.nesdev.org/wiki/PPU_power_up_state)) | Both |
| `mapper_revisions` | `{"mmc3": "bc", "bf909": "bf909x"}` | Board revisions the ROM header can't name: MMC3 (mapper 4) `a`, `bc` or `acclaim`; BF909 (mapper 71) `bf909x` or `bf9097` | All |
| `four_player` | `none` | `none`, `four-score` or `satellite` (the Famicom adapter) | Refuses all but `none`: it applies controllers 1 and 2 only |
| `zapper` | false | Whether a Zapper is in port 2 | Refuses true: input logs carry no Zapper data |
| `concurrent_dpad` | true | Whether opposing D-pad directions can be held together | Refuses false: it applies input exactly as logged |
| `render_video` | false | Whether the PPU draws pixels. Headless, `tetanes-core` detects sprite 0 hits on a separate path | Both |
| `mix_audio` | false | Whether the APU mixes samples | Both |

`gp-nes` never applies Game Genie codes, run-ahead or battery-backed RAM files, so they aren't recorded; adding one is a schema change. Video filtering and APU channel muting only change what's shown and heard, so they aren't recorded either. An arcade recording keeps the defaults here, and nothing reads them: the board powers on one way, as MAME does.

## Index

The store's `index/` directory holds the recordings log. game-printer also kept a SQLite index built from the log and the manifests (`index/recordings.sqlite`, below); gameprinter keeps none and reads the log each time, with the same rows and output.

### Recordings log

`index/recordings.jsonl`, one JSON object per line, defined by `IndexEntry` in `crates/gp-evidence/src/recordings.rs`. It holds what describes a recording without being part of the run. Each line is appended in a single write so that concurrent `gp record` runs can't interleave inside a line. A damaged line hides only itself. Every platform reads the log, WASM included.

| Field | Meaning |
|---|---|
| `id` | The recording's ID |
| `created_at` | RFC 3339 |
| `source` | Optional: the ROM entry's ID in a game's roms.toml, such as `donkey-kong-ju` |
| `frames` | Frame count, for listing without reading the manifest |
| `name` | A short name |
| `notes` | Optional free text |
| `golden` | Optional, default false: true once the diff against the oracle has been triaged and accepted (`gp oracle golden` sets it) |

Several entries can share an ID. Recording the same run under another name adds an entry; recording it again under the same name adds nothing, and marking it golden adds one entry with the same name and `golden` set. `gp recordings list` shows every entry. Finding a recording by ID prefix and `gp replay --all` treat each ID once. Merged, its entries give the first creation time and the latest entry's other fields, and the ID is golden if any entry is.

The log also holds marks, defined by `Mark` in the same file: lines that set something about a recording after it was made, such as `{"mark":"retired","id":"...","at":"2026-10-27T21:00:00Z","note":"superseded"}`. A mark line has a `mark` field and no other fields than `id`, `at` (RFC 3339) and an optional `note`. The kinds are `retired`, `unretired` and `deleted`. For each ID the latest `retired` or `unretired` decides whether it's retired. `gp store gc --retired` appends `deleted` just before it deletes a retired recording's objects; reading the log then ignores every line for that ID up to and including its last `deleted` mark, so the ID is in no listing and its objects are unused. An entry for the ID after the mark (the same run recorded again) brings it back with only the lines that follow. The `deleted` kind (2026-10-27) changes the log's format without a version number, since the log has none and a version field would itself be a line older readers can't parse. A `gp` from before it reports each `deleted` line as one it can't read and skips it. It still sees the recording's earlier `retired` mark, so its `gp replay --all` and `gp recordings list` keep leaving the recording out. Its `gp recordings list --retired` reports the manifest as missing. Its `gp store unreferenced` exits 2 while the line is there. Every reader goes through `gp_evidence::recordings`, which applies the rule: `gp replay --all`, `gp recordings list`, `gp store reindex` and `gp coverage`. Marks are only ever appended, so the log keeps their history, and they change no manifest or entry. A `gp` from before marks existed warns about each mark line as one it can't read and skips it, and its `gp store unreferenced` exits 2 while any mark line is in the log, since it refuses to list unreferenced objects when a log line can't be read.

Golden status describes the run and belongs to the ID. Since it can change after recording, the index holds it rather than the manifest. `gp oracle golden` sets it, once the recording's diff against Mesen has every class triaged and none is a fork bug, by adding an entry with `golden` set whose notes name the stored verdict.

### SQLite index (game-printer)

`index/recordings.sqlite`, defined by game-printer's `crates/gp-index`, format 2 (SQLite's `user_version`). gameprinter has no `gp-index`, never writes this file and ignores one a store game-printer used still holds; `gp store reindex` reads the log and the manifests and writes nothing. The tables are kept here for a reader of an old store. Nothing in it was the only copy, so it can be deleted at any time.

| Table | One row per | Columns |
|---|---|---|
| `entries` | Log line, in log order | `seq`, then the log's fields |
| `recordings` | ID whose manifest could be read | `id`; `schema` as stored, before conversion; `source_kind`, `source_sha256`, `mapper`; `source_id`, the registry ID from the latest entry that has one; `runtime_name`, `runtime_version`, `runtime_commit`; `power_on` and `provenance` as JSON, and `provenance_kind`; `frames`, `checkpoint_interval`; `frame_hash_format`, from the frame hash object's header; `has_trace`, `has_events` |
| `objects` | Store object a recording uses | `recording`, `kind` (`manifest`, `input_log`, `frame_hashes`, `checkpoint`, `trace_fields` for the frame trace's field table, `trace` for its chunks, or `events`), `ord` within the kind, `frame` for a checkpoint, `hash`, and `bytes` when indexed, empty if the object was missing |
| `unreadable` | ID in the log whose manifest is missing or can't be read | `id`, `error` |
| `bad_lines` | Log line that can't be parsed | `line`, `error` |
| `log_state` | The log as last read | `bytes` (-1 for no log), `mtime_ns` |

The index is current when the log's length and modification time equal `log_state`. A command that reads the index first brings it up to date in one transaction, holding SQLite's write lock:

1. Stat the log before reading it, so that a line appended during the update is picked up by the next one.
2. Read the whole log again into `entries` and `bad_lines`.
3. Drop recordings whose lines are gone.
4. Read the manifest of each new ID into `recordings` and `objects`, or into `unreadable`.
5. Store what the stat found in `log_state`.

Manifests are store objects and never change, so a recording's row is never read again. Writers wait up to 30 seconds for each other.

### Unused objects

`gp store unreferenced` lists the store objects that no recording uses. Used objects are those any recording's manifest names, the manifests of the IDs in the log, and objects an entry's notes name as `blake3:<hash>`. It never deletes anything. `gp store gc` deletes those that stayed unreferenced across two runs ([recording.md](../recording.md#deleting-unused-objects)). Objects a line of the coverage index (`<store>/index/coverage.jsonl`) or the validation index (`<store>/index/validation.jsonl`) names are used too, and gc stops while one of their lines can't be read. It keeps its list in `<store>/gc/pending.json` (format 1: the run's time, whether it counted retired recordings, and the object hashes) and takes the advisory lock on `<store>/lock` exclusively; every command holds that lock shared from its first write to the store until it exits.

## Input log

Text, one line per frame. Implemented in `crates/gp-evidence/src/input_log.rs`.

```text
gp-input-log 0
controllers 2
P R......A ........
- R......A ....T...
```

- Line 1 is the format version. Line 2 is the number of controller ports, 1 to 4.
- Each frame line starts with `P` if the game read the controllers during the frame, or `-` for a lag frame.
- Then one field per controller: the buttons in the order Right, Left, Down, Up, sTart, Select, B, A, written as the letter from `RLDUTSBA` when held and `.` when not. This is the FCEUX `.fm2` convention, which makes importing simple.
- Lag frames keep whatever was held.
- Format 1 adds resets: `P!` or `-!` in place of `P` or `-` presses the console's reset button at the start of that frame, before it runs, as FCEUX does for a movie frame's reset command. A runtime applies it before setting the frame's buttons. A log without a reset is still written as format 0, so the logs, and the IDs of the recordings that name them, made before format 1 keep their bytes. Readers take both.
- In memory and in the runtime protocol, a controller is one byte with A in bit 0 through Right in bit 7, the order the NES shift register returns them. Port 1 is the low byte of `input-frame.buttons`.

An arcade game's log is spelt as the MAME oracle's input log ([oracle-mame.md](../oracle-mame.md)), so a capture's log and a clip's are one format:

```text
gp-arcade-input 0
system rbtapper
columns CSRLDUT
600 .......
6 C......
```

- `system` names the MAME set and `columns` the buttons, one letter each from bit 0 (coin, start, right, left, down, up, tap for Tapper).
- Each line is a count of frames and the buttons held through them, the column's letter when held and `.` when not; `#` starts a comment.
- The bits go in controller 1's byte, as the runtime protocol's `buttons` has them. The log keeps no lag marks or resets: every frame reads as polled, and the lag frames `gp record` reports come from the board's own input port reads.

## Frame boundary

A frame runs from the start of vertical blank (the NMI edge) to the next, and every runtime captures frame state there, Mesen included through its scripting hooks. An arcade frame is one 30 Hz video frame of the board, ending at the start of VBLANK as MAME ends it ([recording.md](../recording.md#arcade-recordings)).

On the NES, vertical blank starts at scanline 241, dot 1: the PPU sets the vblank flag on that dot and, if NMI is enabled, raises NMI. The CPU only stops between instructions, so a `gp-nes` frame ends at the first instruction boundary at which the PPU is at or past that dot. The PPU is then a few dots past it: up to 21 for the longest instruction, more if an interrupt sequence follows the instruction in the same step, or an OAM DMA. The same run always stops at the same point. A game's NMI handler therefore runs at the start of the next frame, with that frame's input. Mesen ends its frames by the same rule with its own PPU. The two PPUs are aligned differently, so game-printer's Mesen diff absorbed a boundary that lands an instruction apart or on either side of an NMI entry.

Frame 0 is the power-on state, before any input, with the CPU at the reset vector and the PPU at scanline 0, dot 0. Frame 1 runs from there to the first vertical blank, so it is short: 241 scanlines instead of 262 (about 27,390 CPU cycles instead of 29,781 on NTSC). Every later frame is a whole one. Frame N is the state after N frames have run, so the input log's entry N-1 is the input frame N was run with. A frame's `polled` flag counts controller reads between the boundary before it and the boundary that ends it.

Recordings made before 2026-10-27 used `tetanes-core`'s own frame, which ends at the first instruction boundary after scanline 240 starts. Their frame hashes are format 0 (below) and replay refuses them; record them again.

## Frame hashes

The format is binary and little-endian, and `crates/gp-evidence/src/frame_hashes.rs` implements it.

| Part | Contents |
|---|---|
| Header, 16 bytes | Magic `GPFH`, format version (u32, 1), entry count (u64) |
| One entry per frame, 64 bytes | The BLAKE3 hash of the canonical state (below), then the BLAKE3 hash of the console's 2KB of internal RAM |

There are `frames + 1` entries, for frames 0 to `frames`. Two hashes per frame let replay tell game logic diverging (RAM) from the rest of the console's state diverging.

| Version | State hash | Frame ends |
|---|---|---|
| 0 | `tetanes-core`'s save state, which includes its settings and internal layout | At the first instruction boundary after scanline 240 starts |
| 1 | The canonical state | At the start of vertical blank |

Both versions decode, but a runtime can only check the version it writes, so `gp replay` refuses version 0 with a message to record again.

## Canonical state

The state hash covers a state we define rather than any emulator's save format, so a setting or a patch that changes how the emulator stores itself doesn't change it, and Mesen can produce the same bytes. For NES it is defined by `crates/gp-nes/src/canonical.rs`, field table version 5: an 8-byte header (magic `GPNS`, then the version as a u32), then each field in this order. Numbers are little-endian. A region is a u32 byte count followed by that many bytes, and is empty when the cartridge has none. Version 4 lacked CNROM's register. Version 3 held the open bus with decayed bits cleared and left PRG RAM out on NROM boards without a battery; version 2 lacked the open bus ages and counted cycles in 32 bits; version 1 also lacked the MMC1 and MMC3 registers.

| Fields | Bytes | Contents |
|---|---|---|
| `cpu.a`, `cpu.x`, `cpu.y`, `cpu.s`, `cpu.p` | 1 each | Registers; `p` with bits 4 and 5 as `tetanes-core` holds them |
| `cpu.pc` | 2 | Program counter |
| `cpu.cycles` | 8 | CPU cycles since power-on |
| `cpu.interrupts` | 1 | NMI and IRQ latches: bit 0 NMI asserted, 1 NMI as of the previous cycle, 2 NMI pending as of the previous cycle, 3 IRQ due after this instruction, 4 the same as of the previous cycle |
| `cpu.dma`, `cpu.oam_dma_page` | 1, 1 | DMA in progress: bit 0 DMC fetch, 1 CPU halted, 2 alignment read, 3 OAM DMA pending; and the OAM DMA source page |
| `ram` | 2048 | Internal RAM |
| `ppu.scanline`, `ppu.dot` | 2, 2 | Where the PPU is |
| `ppu.odd_frame` | 1 | Whether this PPU frame is odd (odd frames skip a dot while rendering) |
| `ppu.ctrl`, `ppu.mask`, `ppu.status` | 1 each | $2000 as written, except bits 0-1, which live in `t`; $2001; the three $2002 flags in bits 5-7 |
| `ppu.oamaddr` | 1 | $2003 |
| `ppu.v`, `ppu.t`, `ppu.x`, `ppu.w` | 2, 2, 1, 1 | Internal scroll and address registers |
| `ppu.read_buffer`, `ppu.io_latch` | 1, 1 | The $2007 read buffer and the PPU's open bus latch as `tetanes-core` holds it |
| `ppu.io_latch_age` | 4 | Frames since each set bit of the latch was driven, 0 to 4, three bits per latch bit (bit 0's lowest). A bit not driven for more than 3 frames (age 4) reads as 0 through every register except a palette read, which still returns bits 6 and 7 as held until the bus is next driven, as in Mesen 2. Ages count at the start of scanline 240, and a 0 bit's age is 0 |
| `ppu.nmi_pending` | 1 | The PPU's NMI output |
| `oam` | 256 | Sprite memory |
| `palette` | 32 | $3F00-$3F1F as read, so $3F10, $3F14, $3F18 and $3F1C show the colors they mirror |
| `nametables` | region | Nametable RAM (CIRAM), 2KB on most boards |
| `chr_ram` | region | Pattern tables, only when CHR is RAM |
| `prg_ram` | region | Cartridge RAM at $6000 as the runtime maps it. `tetanes-core` maps 8KB on every NROM board, which most of the boards don't have, but a program can use it, so it's kept |
| `mapper.number`, `mapper.mirroring` | 2, 1 | The iNES mapper number and the current mirroring (0 vertical, 1 horizontal, 2 one-screen lower, 3 one-screen upper, 4 four-screen). Defined for NROM (0), MMC1 (1, MMC1B and later), CNROM (3) and MMC3 (4); a mapper without a definition is refused, and adding one is a new version |
| `mmc1.shift`, `mmc1.shift_count` | 1, 1 | MMC1's serial port: the value it holds so far and how many bits (0 to 4). All `mmc1.*` fields are 0 on other mappers |
| `mmc1.control`, `mmc1.chr0`, `mmc1.chr1`, `mmc1.prg` | 1 each | The four registers as last written, 5 bits each: control (mirroring, PRG mode, CHR mode), CHR bank 0, CHR bank 1, PRG bank with the PRG RAM disable bit |
| `mmc1.last_chr`, `mmc1.write_delay` | 1, 1 | Which CHR register was written last (0 for $A000, 1 for $C000), which picks the PRG bank's high bit on 512KB boards. `tetanes-core` keeps the address of the fifth write and compares it with $C000, so a write to a mirror ($C001-$DFFF) reads as 0 here, as it does in its banking; and the CPU cycles left, 2 down to 0, in which a write is ignored |
| `mmc3.bank_select`, `mmc3.r0` to `mmc3.r7` | 1 each | MMC3's $8000 as written and its bank registers R0 to R7. All `mmc3.*` fields are 0 on other mappers |
| `mmc3.irq_latch`, `mmc3.irq_counter`, `mmc3.irq_enabled`, `mmc3.irq_reload`, `mmc3.irq_pending` | 1 each | The scanline counter: reload value ($C000), count, enable ($E001 and $E000), the reload request ($C001), and the IRQ it's asserting |
| `mmc3.a12_low` | 1 | The A12 edge filter: 0 when A12 hasn't been low since it was last high, otherwise 1 plus the CPU cycles it has been low, capped at 4, so 1 to 5 (a rise after 4 or more clocks the counter). `tetanes-core` counts those cycles in a u32 that wraps after about 40 minutes, and treats a low starting at count 0 as not low, so two states that encode the same can differ in that rare case. `tetanes-core` holds a free-running cycle count here, which the canonical state leaves out. $A001 (PRG RAM protect) isn't emulated, so it has no field |
| `cnrom.chr_bank` | 1 | CNROM's CHR bank as last written to $8000 to $FFFF, all 8 bits (a bank past the CHR ROM wraps as the PPU reads it). `tetanes-core` has no bus conflicts, so it is the value written. 0 on other mappers |
| `apu.pulse1.*`, `apu.pulse2.*` | 21 each | Timer period and counter, duty and sequencer step, length counter (enabled, halt, count), envelope (start, loop, constant volume, volume, divider, decay level), sweep (enabled, negate, reload, shift, period, divider) |
| `apu.triangle.*` | 12 | Timer, sequencer step, length counter, linear counter (reload flag, control, reload value, count) |
| `apu.noise.*` | 16 | Timer, mode, shift register, length counter, envelope |
| `apu.dmc.*` | 21 | Timer, IRQ enable and flag, loop, sample address and length, current address and bytes left, sample buffer and whether it's empty, output level, shift register, bits left, silence |
| `apu.frame_counter.*` | 11 | Mode, step, cycle within the sequence, IRQ inhibit and flag, a pending $4017 write (flag, value, delay) |
| `pad1.index`, `pad1.strobe`, `pad2.index`, `pad2.strobe` | 1 each | Each controller port's shift position and strobe |

What's left out, and why:

- Runtime settings: video and audio output, sample rate, speed, the concurrent D-pad option, turbo timers. None of them change what the console does.
- Master-clock counters and the APU's per-block cycle counters, which `tetanes-core` rebases once a frame for its own bookkeeping.
- The held buttons. They're input, recorded in the input log. `tetanes-core` reads them live when the game reads $4016 or $4017, as if the shift register latched them continuously, so what the game saw shows in RAM or in the controller fields. A changed input on a frame the game doesn't read changes nothing.
- Pipelines that last less than an instruction (the delayed $2001 and $2006 updates, the PPU's tile shift registers, sprite evaluation for the current line). At an instruction boundary these are either settled or determined by the fields above; if the Mesen comparison finds otherwise, they join in a new version.

`canonical::decode` splits a state into its named fields. The frame trace (below) stores the same table as data, so a reader needs no runtime code at all.

## Frame trace

Every frame's canonical state, frames 0 to `frames`, in chunks of `chunk_frames` frames (64 by default). Chunk `i` holds frames `i * chunk_frames` onward, and the last chunk holds what's left. Each chunk is one object; `crates/gp-evidence/src/trace.rs` implements the format.

| Part | Bytes | Contents |
|---|---|---|
| Header | 56 | Magic `GPTR`, chunk schema (u32, 1), first frame (u64), frame count (u32), the BLAKE3 hash of the field table (32 bytes), and the body's length before compression (u32) |
| Body | The rest | Compressed as one LZ4 block (the block format, with no frame and no size prefix; the header has the size) |

The body, once decompressed:

1. The keyframe: its length, then the first frame's state in full.
2. For each later frame, the byte ranges that differ from the frame before: a range count, then for each range the number of bytes skipped since the end of the previous range (or the start of the state), the range's length, and its bytes. Two differing bytes at most two bytes apart go in one range, since a new range costs at least two bytes.

Counts and lengths in the body are unsigned LEB128. Every state in a chunk has the same length, as the canonical state does for one cartridge.

The field table is an object of its own, named by the manifest's `trace.fields` and by every chunk header. `gp-nes` writes the table of the canonical state above, so every NES recording of the same field table version names the same object. An arcade recording's table is gp-arcade's (state magic `GPAR`, version 2: the frame, both Z80s' registers, the five RAMs, the sound latches, both AY-8910s' registers, the input ports, then the BLAKE3 hash of the whole machine's snapshot; `gp trace fields` lists them). A Mesen recording's table was Mesen's own (state magic `GPMS`, version 1, in game-printer's `crates/gp-oracle/src/mesen.rs`): the canonical names for the fields Mesen shows with the same meaning, and other names for the rest. Its frame hashes hash those states, and it has no checkpoints, since only Mesen could restore one. gameprinter reads such a recording from its table alone.

| Part | Contents |
|---|---|
| Header | Magic `GPFT`, table schema (u32, 1), then the magic (4 bytes) and version (u32) that start each state, such as `GPNS` and 5 for `gp-nes` |
| Fields | A count (u32), then for each field in order: the name's length (u8) and the name, the kind (u8: 0 fixed, 1 region) and the width in bytes (u32, 0 for a region) |

`FieldTable::split` splits any state by its table. `gp trace verify` hashes every state in the trace and compares it with the frame hashes, so it needs neither a ROM nor the runtime that made the recording; a whole `gp replay` and `gp replay --all` run the same check after replaying, stale recordings included. Readers check each chunk's header against the frames the manifest gives that chunk before decoding it, and `gp trace frame` and `gp trace diff` hash each state they read from the trace against its frame hash.

Nothing a chunk says is allocated until it's checked against the manifest. Before decompressing, a reader refuses a header whose frames aren't the ones the manifest gives the chunk, and a body length over what those frames can need or over 255 times the compressed bytes (LZ4's largest expansion). A trace chunk must name the manifest's field table, and every state in it must split into that table's fields, with each region at most 4 MiB (`FieldTable::split` refuses a longer one). An event frame holds at most one event per dot of a 312-scanline frame (106,392), and every position must fall inside that frame. A fuzz test in `just check` (`crates/gp-evidence/tests/decoder_fuzz.rs`) feeds the decoders random and damaged chunks, and fails on a panic or on a decode that allocates more than these bounds allow.

### Compression

The format spec draft said zstd. The trace is LZ4 instead, chosen by measurement in game-printer on the trace of its Super Mario Bros. baseline (`baselines/smb1-ntsc-prg0/run-right.toml`, not in gameprinter): 29 chunks whose bodies total 693,990 bytes, from 22.9MB of raw states. Times are for the whole trace, on an M-series Mac, the best of five runs.

| Compressor | Bytes | Compress | Decompress | Builds for `wasm32-wasip2` |
|---|---|---|---|---|
| `lz4_flex` 0.14.0, LZ4 block | 218,814 | 0.5 ms | 0.4 ms | Yes (pure Rust) |
| `ruzstd` 0.9.1, its only implemented encoder level (about zstd level 1) | 269,164 | 4.0 ms | 1.2 ms | Yes (pure Rust) |
| `zstd` 0.14.3 (C libzstd), level 1 | 174,727 | 0.6 ms | 0.3 ms | No |
| Same, level 3 | 169,188 | 0.9 ms | 0.3 ms | No |
| Same, level 19 | 157,140 | 45.8 ms | 0.4 ms | No |

The C library builds for WASI only with a C compiler and a WASI sysroot. With the system `clang` on macOS its build script runs, but the module fails to link (`ZSTD_compress2` is left as an unresolved import), and adding wasi-sdk to every platform and the Docker image costs more than the 22% that zstd level 3 saves. The pure-Rust zstd encoder is larger and slower than LZ4. `gp-evidence` pins `lz4_flex` to an exact version: chunks are named by the hash of their compressed bytes, so a compressor that wrote different bytes for the same data would change recording IDs.

## Event stream

What happened within each frame, in order. The stream is chunked like the trace, so chunk `i` holds frames `i * chunk_frames` onward and the chunks cover frames 0 to `frames`. Frame 0 (power-on) never has events. `crates/gp-evidence/src/events.rs` implements the format, and a hook in the `tetanes-core` fork collects the NES events.

| Part | Bytes | Contents |
|---|---|---|
| Header | 24 | Magic `GPEV`, chunk schema (u32, 1), first frame (u64), frame count (u32), the body's length before compression (u32) |
| Body | The rest | One LZ4 block, as in the trace |

The body has one entry per frame, in order. Each is an event count (unsigned LEB128) followed by that many records.

| Field | Encoding | Meaning |
|---|---|---|
| Kind | u8 | The code from the next table |
| Position | Zigzag LEB128 | The change in `scanline * 341 + dot` from the event before in the same frame; the frame's first event counts from 0. Zigzag maps 0, -1, 1, -2 to 0, 1, 2, 3, since the scanline wraps from 261 to 0 within a frame |
| Address | u16, little-endian | See the next table |
| Value | u8 | See the next table |

Storing the change in position rather than the scanline and dot makes the same writes in consecutive frames the same bytes, which is what LZ4 compresses. For the Super Mario Bros. baseline run it took the stream from 246KB to 58KB.

| Code | Kind | When | Address | Value |
|---|---|---|---|---|
| 0 | `nmi` | The CPU takes the NMI vector | The handler's address | 0 |
| 1 | `irq` | The CPU takes the IRQ vector (a `BRK` isn't logged) | The handler's address | 0 |
| 2 | `sprite0-hit` | The PPU sets the sprite 0 hit flag, rendering or headless | 0 | 0 |
| 3 | `bank-switch` | A CPU write to the cartridge ($4100-$FFFF) changes the PRG or CHR page tables, which covers bank switches and mirroring changes | The write's | The write's |
| 4 | `controller-read` | The CPU reads $4016 or $4017 | $4016 or $4017 | What it read |
| 5 | `controller-write` | The CPU writes $4016, the controller strobe | $4016 | The write's |
| 6 | `ppu-write` | The CPU writes $2000-$2007 (mirrors folded down) or $4014 (OAM DMA) | The register | The write's |
| 7 | `apu-write` | The CPU writes $4000-$4013, $4015 or $4017 | The register | The write's |

Frame N's events happened while frame N ran. Since frames start and end just after dot 1 of scanline 241, events on that scanline can open a frame or close it. The vblank NMI is taken either just before a frame's end or just after it, depending on the instruction it interrupts. A frame can hold up to two NMIs, or none, and the same run always splits them the same way.

The stream leaves out the following. Adding a kind is a new chunk schema.

- Reads of PPU and APU registers. Only controller reads are logged.
- OAM DMA's 256 writes to $2004. The $4014 write that starts it stands for them.
- Bank switches that PPU fetches make, such as MMC2's CHR latches, and those of boards that serve reads themselves.
- An NMI that hijacks a `BRK`.

A Mesen recording (`gp oracle record`) logs `nmi`, `irq`, controller reads and writes, and PPU and APU writes, leaving out the same things, at the position Mesen's callback saw; it logs no sprite 0 hits or bank switches, since Mesen's script interface raised no event for them.

An arcade recording's stream has the same bytes in a geometry of its own: positions count Tapper's 481 lines of 512 dots (`Geometry::MCR`, the last line holding what a CPU runs past the frame's end), and the kinds are the board's: `irq` (the main CPU took a CTC interrupt: the handler, and the vector), `sound-irq`, `port-read` (an input port and what the game read), `sound-latch`, `watchdog` and `ay-write` (chip times 256 plus register). The manifest's source says which geometry a stream counts in.

Replay checks a recording's events frame by frame when it has an event stream, after RAM, state and polling, and reports a difference as its own divergence kind. `gp events` serves events as stored, unchecked, so replay is what checks them.

## Model traces and events

A model run (`source.kind` `model`) keeps a trace and an event stream in the same chunk containers, so the store and `gp trace` read them alike.

The trace's states aren't the model's canonical state, whose length changes with the bodies it holds. A state is a view by model path: the 8-byte header (`GPSP`, version 1) and then one fixed-width field for each path the run ever names, in the order the run first names them. A number is 8 bytes (little-endian, signed, fixed-point in 65536ths), a flag 1 byte and a name 24 bytes of UTF-8 padded with zeros. An entity instance has an extra `<instance>.present` flag; an instance that isn't in the world on a frame has it false and its fields zero. Which numbers are fixed-point is by name (`x`, `y`, `vx`, `vy` of entities and cameras), since a field table gives widths only. The frame hash's `state` is the BLAKE3 hash of the view, so `gp trace verify` works unchanged; the `ram` hash is the canonical state's hash, which checkpoints restore to.

The event stream's chunk schema is 2. The header is schema 1's (`GPEV`, schema, first frame, frame count, body length) and the body one LZ4 block: a name table (a count, then each name as a length and UTF-8 bytes, all unsigned LEB128), then for each frame an event count and that many records of the name's index in the table and the entity's index plus one (0 for none), both unsigned LEB128. A reader of schema 1 refuses these chunks and a reader of schema 2 refuses schema 1's. `crates/gp-evidence/src/model_events.rs` implements it, and `crates/gp-interp/src/modeltrace.rs` the trace's view.

## Checkpoints

A checkpoint is the runtime's full serialized state at a frame, stored as an object: frame 0 and every `checkpoint_interval` frames (60 by default). Any frame is reachable by restoring the nearest earlier checkpoint and replaying at most that many frames. Snapshots are opaque and only the runtime that made one can restore it, so a checkpoint is only valid for the `runtime` named in its manifest.

For `gp-nes`, a snapshot is `tetanes-core`'s save state, 21,518 bytes for Super Mario Bros. For `gp-arcade` it is the machine's snapshot, and a recording from another build of gp-arcade replays from power-on only. Snapshots are only for checkpoints; the frame hashes don't use them. `tetanes-core` releases every button when it loads a state, so restoring frame N also sets the controllers to the input log's entry N-1. That makes the restored save state byte for byte the one recorded, not only its canonical state.

## Sizes

Measured in game-printer on 2026-10-27 on the 1,800 frames of its Super Mario Bros. baseline, recorded with `gp record` defaults:

| Part | Bytes | Per frame |
|---|---|---|
| Frame trace, 29 chunks and the field table | 223,432 | 124 |
| Event stream, 29 chunks, 95,725 events | 57,819 | 32 |
| Checkpoints, 31 | 667,058 | |
| Frame hashes | 115,280 | 64 |
| Whole recording | 1,092,570 | |

Without the trace and events the same recording is 806,784 bytes, so the two add 35%.

## Instruction traces

Not stored with recordings. Produced on demand for a range of frames by restoring the nearest checkpoint and replaying with instruction tracing on (`gp trace instructions`). The format is text, one line per instruction in the layout of nestest.log, with `;` comment lines for each frame's input and for interrupts; [runtime-protocol.md](../runtime-protocol.md#instruction-traces) describes it. Frame N in a trace is numbered as everywhere else: the frame that produces state N, run with input log entry N-1, whose events the event stream lists under N. Tracing reads the console between instructions and changes nothing, so a traced replay ends in the recorded state.

## Versioning

Every breaking change bumps `schema` and ships a converter, so existing recordings never need to be re-recorded because the format moved. `RecordingManifest::from_json` reads every supported schema and converts it to the current one in memory. The stored bytes don't change, so a recording keeps its ID, which is the hash of those bytes.

| Schema | Change | Converting from the previous one |
|---|---|---|
| 0 | First version | |
| 1 | The manifest holds only what was run. `created_at`, `notes`, `golden`, `source.name` and the `scripted` provenance's `name` are gone; `power_on` gained every field after `ram_init` | The removed fields are dropped (the index already had the time and name). The new `power_on` fields get the values `gp-nes` always used under schema 0, which are today's defaults |
