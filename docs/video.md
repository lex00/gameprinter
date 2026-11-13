# Videos of recordings

Brought from game-printer's docs/video.md with `gp video`; an arcade recording's video is the last section.

`gp video` replays a stored recording with video rendering on and writes every frame's picture to an animated PNG, with the frame number, the input and the frame's events drawn below the picture. It's for looking at a run, and for attaching to a bug report or a pull request.

```sh
gp video a85abf91                                  # to $GP_HOME/videos/a85abf91154f.png
gp video a85abf91 -o ~/Desktop/run.png --force     # anywhere outside a git work tree
gp video a85abf91 --frames 300..=420 --scale 3     # a window of frames, three times the size
gp video a85abf91 --no-overlay --scale 1           # only the picture, 256x240
```

`--theme DIR` draws the picture with a theme ([themes](formats/theme.md)), repeatable, bottom first. A recording's pixels carry no roles, so a theme's role rules don't apply there; the rest of it does. Each frame is still checked against the recording before the theme draws, so a themed video matches as an unthemed one does. To see role rules on a game, draw the model's run instead: `gp theme video MODEL --baseline FILE --themes -o OUT`.

`--frames` takes `N`, `A..B` (B left out), `A..=B` or `A..`, like `gp events`. Frame 0 is power-on and has no picture, so a range starts at 1 or later. To start later than frame 1, the replay restores the nearest checkpoint before the window and runs headless up to it, then turns video on. `--scale` (1 to 4, default 2) repeats each NES pixel that many times both ways. A recording whose ROM isn't in `$GP_ROM_DIR` needs `--rom`.

## What the overlay shows

The band under the picture has one line for the frame and controller 1, a line for controller 2 when the window uses it, and a line of event counts.

| Item | Meaning |
|---|---|
| `F 001200` | The frame: the picture is what the console drew while running it |
| `P1 U D L R SE ST B A` | The buttons held during the frame, bright when pressed |
| `LAG` | The game didn't read the controller during the frame |
| `DIFF` | The frame differs from the recording (below); the picture also gets a red border |
| `NMI`, `IRQ`, `S0`, `BNK`, `PPU`, `APU` | How many NMIs, IRQs, sprite 0 hits, bank switches, PPU register writes and APU register writes the frame had |

Short marks at the edges of the picture show where in the frame some events happened: a yellow mark at the left edge on the scanline of each sprite 0 hit, a magenta one for each IRQ, and a cyan mark at the right edge for each bank switch. Super Mario Bros. shows its sprite 0 hit under the status bar every frame of play.

## Checked while it draws

Every frame is checked against the recording as it's drawn. The RAM and canonical state hashes must equal the recorded frame hashes, the lag flag must match the input log, and the events must match the recording's event stream. The recording was made headless, and `tetanes-core` finds sprite 0 hits on a separate path when it renders, so this also checks that the two paths agree for the run. A frame that differs is marked `DIFF`, and the command exits 1 naming the first one. A recording made by another runtime is replayed from power-on instead of from checkpoints, and differences are marked but don't fail the command, as `gp replay` treats such recordings.

## Where videos go

A video of a commercial game shows its graphics, so it's private, like the recording it comes from. `gp video` writes to `$GP_HOME/videos/` by default and refuses any path inside a git work tree, as `$GP_STORE` does. It refuses to replace an existing file without `--force`, and it writes to a temporary file beside the output and renames it at the end, so a failed run leaves no half-written video.

## Format

The output is an animated PNG (APNG): lossless, so pixel art and the overlay's text stay sharp, and played by browsers, so it shows in a pull request or an issue. Its frame delay is 1000/60099 of a second for NTSC (60.099 frames a second, the console's 60.0988 to within 0.0004%) and 1000/50007 for PAL. It plays once. Each frame after the first stores only the rectangle that changed since the frame before, compressed with deflate by the `png` crate. The writer is `crates/gp-cli/src/theme_apng.rs`, which `gp theme video` uses too.

game-printer measured it on 2026-10-27 with its debug build (`png` and its compressors are optimized even there):

| Recording | Frames | Scale | Size | Speed |
|---|---|---|---|---|
| Moss Hop, finish-level | 300 | 2 (512x520) | 0.51 MB | 136 frames/s |
| Super Mario Bros., `68fdf735` (run-right) | 1,800 | 2 (512x520) | 2.4 MB | 128 frames/s |
| Super Mario Bros., `68fdf735` | 1,800 | 1 (256x260) | 1.7 MB | 131 frames/s |

That's about 1.3 MB per recorded minute of Super Mario Bros. at scale 2. Rendering runs at about twice real time in the debug build.

In gameprinter, the first 300 frames of the Donkey Kong recording 24640121d314 give the same file, byte for byte, as game-printer's `gp video`.

## On an arcade recording

On a recording of an arcade romset (Tapper's), `gp video` replays the board on gp-arcade from the nearest checkpoint and draws its picture as the board made it, 512 by 480, so `--scale` is 1 by default there. Each frame is checked as on the NES: the state against the recorded frame hashes and the board's events against the recording's event stream. The board reads its inputs as ports, not by polling a controller, so there is no lag flag to check or mark. The band has two lines:

| Item | Meaning |
|---|---|
| `F 001200` | The frame |
| `C ST R L D U T TL` | The board's inputs during the frame, bit 0 first (coin, start, right, left, down, up, tap, tilt), bright when held |
| `DIFF` | The frame differs from the recording; the picture gets a red border |
| `IRQ`, `SIRQ`, `LATCH`, `AY`, `PORT`, `WD` | The main CPU's interrupts, the sound CPU's, the sound latch writes, the AY-8910 register writes, the input port reads and the watchdog feeds of the frame |

A magenta mark at the left edge shows the line of each main CPU interrupt, and a cyan one at the right edge each sound CPU interrupt. The frame delay is 1/30 of a second. `--theme` is the NES's and refused. On Tapper's five-level recording (c0130f81267d), frames 1,000 to 1,299 draw in 7 seconds with the debug build, every frame matching the recording.

`gp mechanic video GAME MECHANIC CLIP` records a session's clip (`gp record`) and draws it with `gp video`, on either platform, after checking the clip's start against the state it was saved with.
