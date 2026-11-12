# Recordings

A recording is a run of the original kept in the store (`$GP_HOME/store`): the ROM's hash, the power-on state, the input log, a checkpoint every so many frames, every frame's state hash, and unless left out a frame trace (every frame's whole state) and an event stream (interrupts, sprite 0 hits, bank switches, controller and register writes). gp-nes is deterministic, so the input log and the ROM are the run; everything else lets a reader start anywhere and check what it reads. The commands came from game-printer with their flags and outputs (`docs/recording.md` there has the measurements); gp-evidence holds the store and gp-nes the recording code.

```sh
gp record --source donkey-kong-ju --input dk.input --name dk-tas
gp replay 3b27                     # every frame checked against the recording
gp replay --all                    # every recording whose ROM is in $GP_ROM_DIR
gp query 3b27 --frame 1000 'ram[0x0400..0x0410]' 'cpu.pc'
gp run-until 3b27 'ram[0x0058] == 2' --show 'ram[0x0058]'
gp trace frame 3b27 500 --field cpu,ppu
gp trace diff 3b27 482d            # the first frame whose state differs, and the fields
gp trace fields 3b27
gp trace verify 3b27               # the trace against the frame hashes
gp trace instructions 3b27 --frames 100..102
gp events 3b27 --frames 300..320 --kind nmi,controller-read
gp recordings list
gp recordings show 3b27
```

`--source` names a ROM entry of a game's `roms.toml` in the games checkout (`$GP_GAMES_DIR`, or a `games` folder beside the current one), and the ROM is found in `$GP_ROM_DIR` by its hash; `--rom FILE` runs a file instead. A recording's ID is the BLAKE3 of its manifest, so recording the same run again gives the same ID, and any unique prefix of four or more characters opens it. `query` and `run-until` read the trace when the recording has one this runtime made, and otherwise restore the nearest checkpoint and replay; either way they exit 1 if the state they reach isn't the one recorded.

`gp recordings list` and `show` read the store's recordings log and the manifests it names. game-printer kept a SQLite index of the log (gp-index); gameprinter scans the log instead, with the same rows and output.

Not here yet: `gp record --model` and the replay of a model's recording (the new world runs a model with hooks through its game's module), `gp recordings retire|unretire`, `gp video` and the `gp store` commands.

## Movies

`gp movie` imports FCEUX `.fm2` and BizHawk `.bk2` movies as input logs. A movie names the ROM it was made on by hash (FCEUX's MD5 of PRG and CHR, BizHawk's SHA-1), and the import refuses one made on another revision with exit 2 and the revision it names.

```sh
gp movie hashes ~/roms/nes/donkey-kong.nes          # the hashes movies use, checked against roms.toml
gp movie import tas.fm2 --source donkey-kong-ju --out dk.input
gp movie import tas.fm2 --source donkey-kong-ju --record --url https://tasvideos.org/4151S
gp movie diagnose 3b27
```

An import keeps the buttons only and marks every frame polled; `gp record` (or `--record`) replaces those marks with what the game read, so lag frames come from gp-nes and not from the movie's emulator. `--shift N` moves the inputs later (or earlier, negative) when the game took another number of frames to boot in the movie's emulator; `--ram-init fceux` powers on with FCEUX's RAM fill, which FCEUX movies were made with. `gp movie diagnose` lists input changes the game never read: a TAS wastes no input, so the first one marks where the replay left the author's path. There is no default source: give `--source` or `--rom`.

On 2026-11-11 the Donkey Kong (JU) TAS 4151S (FCEUX) imported and recorded with no wasted input, and the old and new tools gave the same recording ID.

## Arcade recordings

An arcade romset records the same way, by `--rom tapper.zip` or a `--source` whose roms.toml entry is a romset, with an arcade input log (`gp-arcade-input 0`, docs/oracle-mame.md). gp-arcade runs it, and every command above reads it, except `gp movie diagnose`, which reads lag marks an arcade log doesn't have. What differs from an NES recording:

| | NES | Arcade |
|---|---|---|
| Source in the manifest | `nes-rom`, the headerless SHA-256 and mapper | `romset`, the set's identity and MAME name (`tapper`) |
| Runtime | `gp-nes` | `gp-arcade`, its version with the board's and the recording's (`0.1.0 mcr 2 rec 3`) |
| Frame | one NES frame, to the start of vblank | one 30 Hz video frame, to the start of VBLANK, as MAME ends it |
| State in the trace | gp-nes's canonical state | `GPAR` 2: the frame, both Z80s' registers, the five RAMs, the sound latches, both AY-8910s' registers, the input ports, then the BLAKE3 hash of the whole machine's snapshot (`gp trace fields` lists them) |
| Frame hashes | the canonical state, and internal RAM | the state above, and the oracle's RAM record (work, sprite, video, palette and sound RAM) |
| Events | `nmi`, `irq`, `sprite0-hit`, `bank-switch`, `controller-read`, `controller-write`, `ppu-write`, `apu-write` | `irq` (the main CPU took a CTC interrupt: the handler, and the vector, whose bits 1 and 2 are the CTC channel), `sound-irq`, `port-read` (an input port and what the game read), `sound-latch`, `watchdog`, `ay-write` (chip × 256 + register) |
| Event position | scanline and dot, 312 × 341 | line and dot of the 30 Hz frame from its start, 481 × 512 (the last line holds what a CPU runs past the frame's end) |
| Lag frames | from the game's controller reads, kept in the log | from the board's input port reads, reported by `gp record`; the log keeps no marks, so replay doesn't compare them |
| `--ram-init`, `--region` | power-on choices | unread: the board powers on one way, as MAME does |

`gp trace instructions` on an arcade recording prints both Z80s, one line per instruction in the order of their times (the scheduler runs each CPU for a timeslice in turn; the lines are put back in time order), with `; frame N, controls ...` before each frame and `; IRQ M vector D6` (or `S`) before a handler's first instruction:

```text
; frame 700, controls .......
S 04E3  C9 DD 7E 09                           AF:0400 BC:0053 DE:0353 HL:8005 IX:8199 IY:8043 SP:83F7 LINE:  0, 18 CYC:46600005
; IRQ M vector D6
M 00D8  FD E5 DD E5                           AF:0093 BC:0000 DE:0006 HL:2328 IX:E4A7 IY:E2E6 SP:E7F6 LINE:  0, 35 CYC:116500024
```

Each line is the state before the instruction: the CPU, its address, the four bytes from there, the registers, the line and dot of the frame as the events count them, and the CPU's cycles since power-on. It comes from gp-arcade's instruction hook (`Tapper::set_trace`), which changes no state. One Tapper frame is about 15,000 main and 6,000 sound instructions and 35 interrupts.

`gp movie import` also reads a MAME `.inp` (made with `mame -record`): it names its set by MAME's name rather than a hash, so it must be the target romset's set, and it becomes an arcade log. Record N of a `.inp` holds the ports the game reads during frame N+1, which is the log's entry N. Only what the log can say is taken (coin 1, start 1, player 1's stick and tap); a `.inp` that presses anything else, or has the DIP switches off their defaults, is refused in words. MAME 0.276's layout is in `crates/gp-arcade/src/inp.rs`.

```sh
gp movie import short.inp --rom ~/roms/arcade/tapper.zip --out short.input
gp record --rom ~/roms/arcade/tapper.zip --input short.input --name tapper-short
gp replay ad29
gp trace diff ad29 e12a
gp events ad29 --frames 700 --kind port-read,sound-latch
```

On 2026-11-12 a 1,282-frame Tapper run made in MAME 0.276 (`-record`) imported to the same log MAME played, plus the one frame MAME records after the last; both recordings replayed frame-exact, `gp trace diff` found every frame's state equal to frame 1282, and gp-arcade matched MAME's own capture of the run on all 1,283 frames (RAM, input ports and picture).
