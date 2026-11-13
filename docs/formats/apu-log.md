# APU register log

Brought from game-printer's docs/formats/apu-log.md (port wave 4).

Version 1. Every write a recording made to the NES's APU registers over a range of frames, as `gp sound capture` writes it (`apu.log`) from a recording's event stream or a replay with the event log on. It's an observe-only output of replay: nothing about the replay changes, and BEHAVIOUR_VERSION doesn't either. A log is game content, so it lives with the recording's derived data under `$GP_HOME/work/sound/<recording>/<capture>/` and never in the repo. `gp_sound::log` reads and writes it; tests use made-up logs. `gp interp run --apu-log FILE` writes one from a model's hooks (a game's sound driver kept as state), untimed, with every write at cycle 0.

It's text, one item a line:

```text
gp-apu-log 1
region ntsc
source <the ROM's headerless SHA-256>
recording <the recording's ID>
frames 1 2401
timed true
sample c000 aa55...
# frame cycle register value
120 2398 15 0f
120 2410 00 bf
```

| Line | Meaning |
|---|---|
| `gp-apu-log 1` | The header, first; another version is refused |
| `region ntsc` or `pal` | The clock and frame length the cycles count in |
| `source`, `recording` | Optional: where the log came from |
| `frames A B` | The first frame and the frame after the last. Required |
| `timed true` or `false` | Whether cycles are measured. A log built from per-frame write lists (no timing) has every write at cycle 0 in its frame's order |
| `sample ADDR HEX` | Optional: DMC sample bytes at a CPU address (hex), when capture could read them |
| `F C RR VV` | A write: frame `F` (decimal), CPU cycle `C` within the frame (decimal), register `RR` as its offset from $4000 in hex (00 to 13, 15 or 17), value `VV` in hex. Writes are in order; a register outside the APU's or a frame outside `frames` is refused |
| `#` | A comment |

A frame starts where gp-nes frames end, at the start of vertical blank (scanline 241, dot 1; docs/formats/recording.md). A write the PPU saw at scanline `s` and dot `d` is `((s - 241) mod lines) * 341 + d - 1` dots into its frame (`gp_sound::log::cycle_in_frame`). A CPU cycle is 3 dots on NTSC and 3.2 on PAL. Frame `n` of the log starts at CPU cycle `floor(n * dots_per_frame / 3)` on NTSC (262 lines of 341 dots), so the synthesizer and capture place every write on one timeline.
