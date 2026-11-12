# The MAME oracle

Our own arcade emulators get held to MAME frame by frame, the way gp-nes was held to Mesen in the old repo. This is the reference side: MAME in a pinned Docker image, playing an input log on a romset from power-on and writing what every frame shows. The first game is Tapper, MAME's `rbtapper` set.

## The licence boundary

MAME is GPL-2.0+ as a whole. Here it runs only as an external program, in its own image, installed from Debian's package. Nothing in this repo links, vendors or commits MAME code. The capture script, `scripts/oracle/mame-capture.lua`, is ours: MAME loads it at run time.

ROMs stay where the owner keeps them. The romset zip is mounted read-only into the container for one run and never goes into the image or a repo. Everything a capture writes (RAM, pictures, hashes) is derived from the ROM, so it goes under `$GP_HOME/work/oracle/`, and `mame-record` refuses any other place.

## Running it

| Step | Command |
|---|---|
| Build the image, once per architecture | `just mame-build`, or `just mame-build linux/amd64` |
| Run a capture | `just mame-record ~/Downloads/rbtapper.zip scripts/oracle/inputs/tapper-first.input tapper-first` |
| Fewer or no PNGs | add a fourth argument: `90` for one every 3 seconds, `0` for none |
| The speakers too | add a fifth argument, `1`: MAME records what reaches the speakers as `sound.wav` (48 kHz, 16-bit stereo, about 11 MB a minute) |

OUT is a folder name under `$GP_HOME/work/oracle/` that doesn't exist yet. The recipe prints the frame count, the time taken and the number of input mismatches (see below). To check determinism, run the same log twice into two folders and compare their `SHA256SUMS` files.

Neither recipe is part of `just check`.

## The image

| | |
|---|---|
| Dockerfile | `docker/mame/Dockerfile`, tagged `gameprinter-mame:0.276-arm64` or `-amd64` |
| Base | `debian:trixie-slim`, pinned by the digest of its multi-arch index |
| MAME | Debian 13's own package, `mame` and `mame-data` at `0.276+dfsg.1-1+deb13u1`. MAME 0.276 lists `rbtapper` ("Tapper (Root Beer)") |
| Other packages | Whatever `mame` depends on, from snapshot.debian.org at `20261104T212339Z`, the timestamp the base image names. So every package is the same version whenever the image is built |
| Size | About 940 MB. Debian's `mame` pulls in SDL, Qt and Mesa even though nothing here draws to a screen |
| Headless | `-video none -sound none`, with SDL's dummy video and audio drivers. No host input devices (`-keyboardprovider none` and the like), so only the script presses anything |

Building from source with only the MCR driver would make a smaller image, but Debian's package was recent enough and runs Tapper, so that's what this uses.

## How a run is pinned

| What | How |
|---|---|
| Configuration | `-noreadconfig` reads no ini files, `-noplugins` loads no plugins (so no hiscore plugin), `-nocheat`, `-noautosave` |
| cfg and NVRAM | Both folders are a tmpfs inside the container, empty at the start of each run. Tapper's work RAM at `e000` is battery-backed NVRAM; with no file, MAME fills it with `ff` (the driver's `DEFAULT_ALL_1`) |
| DIP switches | mcr.cpp's defaults: demo sounds on, upright cabinet, one coin meter. Nothing can change them (no cfg, no ini), and the script checks each one against its default and stops if one differs. Port `ip3` in every frame record holds the switch bank as the game reads it |
| Random numbers | MAME's own generator starts from a fixed seed (`0x9d14abd7` in `running_machine`), and Tapper's driver doesn't use the wall clock. The only clock MAME reads is for real-time-clock chips, and the MCR board has none |
| Speed | `-nothrottle -frameskip 0`: MAME runs as fast as it can and draws every frame |

## The input log

gameprinter's own input log is NES pads (`RLDUTSBA`), which can't say "coin". So the oracle has a small text format of its own:

```text
gp-arcade-input 0
system rbtapper
columns CSRLDUT
900 .......
6 C......
54 .......
6 .S.....
20 ......T
```

- The first line is the format and its version.
- `system` is MAME's name for the set, and must match the zip's name.
- `columns` lists the controls, which must be the script's list for that system. Tapper's are coin 1, start 1, right, left, down, up and tap.
- Each other line is a frame count, then one character per column: the column's letter when held, `.` when not.
- `#` starts a comment.

The lines run from frame 1. Frame 1 must hold nothing (see below). Player 2's controls, coin 2, tilt and service aren't in version 0.

## What a frame is

MAME ends a video frame when the primary screen's VBLANK starts. At that point `video_manager::frame_update` finishes the screen's picture and calls the machine's frame notifiers. Tapper's screen runs at 30 Hz, so a capture has 30 frames a second.

- Frame 0 is power-on. The script runs at emulated time zero, after the reset and before any instruction.
- Frame n is the state at the n-th frame notifier, after emulating from the end of frame n-1. Frame 1 is a little shorter than the rest, since the first VBLANK comes before a full period has passed.
- MAME's input system has its own frame notifier, which runs before the script's. Only that notifier copies a value the script sets into the ports the game reads. So the script sets the log's frame n+2 at the end of frame n, and the game sees it during frame n+2. Nothing set by a script can reach frame 1, so the log's frame 1 must be empty.
- Each record holds the input ports as MAME had them during that frame, read back from MAME, not copied from the log. `buttons` is those ports turned back into the log's letters. If it differs from the log, `input_mismatches` in `capture.txt` counts it. A coin the game has locked out, for example, wouldn't reach the port.

## The output

A folder under `$GP_HOME/work/oracle/`:

| File | What |
|---|---|
| `capture.txt` | MAME's version, the system, the frame count, the refresh rate, the picture size and hash, the DIP check, the number of input mismatches, and the layout of `ram.bin` |
| `frames.tsv` | One line per frame, with a header line (columns below) |
| `ram.bin` | One fixed-size record per frame, frame 0 first, 5,760 bytes each (layout below) |
| `ay.tsv` | One line per frame, with a header line: the frame, then the sound CPU's writes to its two AY-8910s in that frame, in order (below) |
| `sound.wav` | With the fifth argument only: the speakers, left and right, as MAME's `-wavwrite` writes them |
| `shots/frame-NNNNNN.png` | The picture of every Nth frame, from frame 0, written by MAME at the screen's native 512x480 |
| `SHA256SUMS` | A digest of every file above, to compare two runs |
| `mame.log` | MAME's own output. Left out of `SHA256SUMS` |

The columns of `frames.tsv`:

| Column | What |
|---|---|
| `frame` | 0 for power-on, then 1, 2, ... |
| `time` | MAME's emulated time at the frame's end, as seconds and attoseconds (`0.033...`) |
| `buttons` | The ports during this frame, in the log's letters; `-` for frame 0 |
| `ip0` to `ip4` | The SSIO input ports as the game reads them during this frame, in hex, active low. `ip0` is coins, starts, tilt and service; `ip1` is player 1; `ip2` player 2; `ip3` the DIP switches; `ip4` unused |
| `picture` | A hash of the frame's picture (below) |
| `main_pc`, `ssio_pc` | The two Z80s' program counters at the boundary. These depend on where MAME's scheduler stops each CPU, so they're for reading, not for comparing |
| `shot` | The PNG's path, or `-` |

The layout of each `ram.bin` record:

| Offset | Size | What |
|---|---|---|
| 0 | 2048 | Main CPU `e000-e7ff`, work RAM (the NVRAM) |
| 2048 | 512 | Main CPU `e800-e9ff`, sprite RAM |
| 2560 | 2048 | Main CPU `f000-f7ff`, video RAM |
| 4608 | 128 | Main CPU `f800-f87f`, palette RAM, read from its memory share because the bus can't read it |
| 4736 | 1024 | Sound CPU `8000-83ff`, the SSIO board's RAM |

The main board's ranges are mcr.cpp's `cpu_91490_map`; the sound board's is midway_sound.cpp's `ssio_map`.

**The AY writes.** A write tap on the sound CPU's program space sees every write to `a000-bfff`. midway.cpp's `ssio_map` puts the first AY at `a000` and the second at `b000`, each with its address latch at offset 0 and its data latch at offset 2, mirrored over `0ffc`, so the tap keeps writes whose offset ends in 0 or 2. Each is written as the chip, `a` or `d` and the value in hex, a space between: `0a07 0d38` selects the first chip's register 7 and writes `38` to it. A frame with none has `-`. Captures made before the tap have no `ay.tsv`.

**The picture hash.** The script asks MAME for the frame as 32-bit pixels, `0xAARRGGBB`, in rows from the top, at 512x480. It then takes FNV-1a over 64 bits, one pixel at a time, using each pixel's 24-bit `0xRRGGBB` with alpha left out. h starts at `0xcbf29ce484222325`, and for each pixel h = (h XOR rgb) * `0x100000001b3`, modulo 2^64. It works on whole pixels, not bytes, so our emulator can compute the same number from its own picture without matching MAME's pixel format.

## Holding gp-arcade to it

`crates/gp-arcade` is our own Tapper. Its compare example plays a capture's input log and checks every frame's RAM, input ports and picture hash against the capture, stopping at the first difference and saying what it is:

```text
cargo run --release -p gp-arcade --example compare -- \
    --rom ~/Downloads/rbtapper.zip \
    --capture ~/.gameprinter/work/oracle/tapper-first-a \
    --log scripts/oracle/inputs/tapper-first.input
```

When the capture has an `ay.tsv`, the AY writes are compared too, frame by frame. `--all` runs to the end and counts the frames that differ in each way. `--png DIR --at N` writes our picture of frame N beside MAME's shot and a third panel with the differing pixels in red; DIR goes outside the repo, since the pictures show the game. `--dump N` prints frame N's differing RAM rows and both CPUs' registers, and `--trace N` every instruction both CPUs start in frame N.

| Capture | Input log | Frames | Result |
|---|---|---|---|
| `tapper-first-a` | `tapper-first.input` | 1511 | all match |
| `tapper-deaths-a` | `tapper-deaths.input` | 6095 | all match |
| `tapper-first-sound` | `tapper-first.input` | 1511 | all match, AY writes too (3,500 writes) |
| `tapper-m3-gameover-ay` (set `tapper`) | `tapper-gameover.input` | 5351 | all match, AY writes too (18,212 writes) |
| `tapper-m3-five-ay` (set `tapper`) | `tapper-five-levels.input` | 10934 | all match, AY writes too (109,838 writes) |
| `tapper-m3-eight-ay` (set `tapper`) | `tapper-eight-levels.input` | 18001 | all match, AY writes too (342,346 writes) |

The `tapper-*` plays of milestone 3 were made by gp-arcade's autoplay example, a player that reads the board's RAM and decides each frame's presses: coin and start, then serving, catching mugs and taking tips. tapper-eight-levels clears eight levels (the saloon, the sports bar and three punk bar levels) and both bonus rounds. The speakers on tapper-five-levels (`tapper-m3-five-sound`, 6 minutes) differ from MAME's WAV by at most one step of 16 bits, as on tapper-first.

The sound example plays a capture's log with the sound on and compares the speakers with the capture's `sound.wav`:

```text
cargo run --release -p gp-arcade --example sound -- \
    --rom ~/Documents/roms/arcade/rbtapper.zip \
    --capture ~/.gameprinter/work/oracle/tapper-first-sound \
    --log scripts/oracle/inputs/tapper-first.input
```

On `tapper-first-sound`, 2,415,360 samples a speaker (50.3 seconds), 12 of the 4,830,720 differ, each by one step of 16 bits; the rest are equal. gp-arcade makes the sound the way MAME 0.276 does: each AY's 250 kHz samples with a write landing at the first sample at or after its time, MAME's 50 Hz stream updates with the gains as they stand at each, its default resampler to 48 kHz, and its 16-bit mix. The one-step differences look like float rounding in the order the mixer adds the channels. One thing was found only by comparing: MAME's speakers get each 48 kHz sample one sample after the resampler makes it, so gp-arcade holds each back one sample too.

`cargo test -p gp-arcade` runs the comparisons, and the sound one on `tapper-first-sound`, when the romset and the captures are present, and skips them otherwise.
