# Songs

Brought from game-printer's docs/formats/songs.md (port wave 4).

The `gp-songs` format holds songs and sound effects as notes (`gp_sound::song`). It's the one general representation every source writes and every player reads. A model's `songs` and `sound-effects` slots hold one (docs/model.md). Captured songs are game content and stay outside the repo, and tests use made-up songs.

The file is JSON. The reader takes 0.1 and 0.2 with any patch, and checks that every note's instrument exists and fits its channel. 0.2.0 added an arcade board's AY-8910 channels and voices, the set's `clock` and a note's `period` (below), so a theme's own songs can target an arcade board; a 0.1 file reads as it always did. A file that says 0.1 and holds any of them is refused. A writer says `0.1.0` while a set holds nothing 0.2 added, so a game package built with an older kit still reads it, and `0.2.0` once it does.

| Field | Meaning |
|---|---|
| `format`, `version` | `"gp-songs"`, `"0.1.0"` or `"0.2.0"` |
| `region` | `ntsc` or `pal`: the clock an APU track's periods are in |
| `clock` | 0.2: the AY-8910's clock in hertz an AY track's periods are in, when it isn't 2,000,000 (Midway's SSIO board) |
| `instruments` | Shared by every song of the set (below) |
| `songs` | Music and effects (below) |
| `samples` | DMC sample bytes by address, when they were read: `[{addr, bytes}]` |

An instrument says what a note does frame by frame from its start; each envelope's last value holds for the rest of a longer note.

| Field | Meaning |
|---|---|
| `name` | Unique in the set |
| `voice` | `pulse`, `triangle`, `noise` or `dmc`: the kind of channel it plays on; on an AY (0.2) `tone`, `ay-noise` or `envelope` |
| `role` | `lead`, `bass`, `harmony`, `drums` or `sfx`, for sound banks and themes |
| `volume` | 0 to 15 a frame (the triangle sounds while it's above 0); on an AY tone, 16 hands the loudness to the chip's envelope |
| `duty` | Pulse duty 0 to 3 a frame, or the noise mode (1: the short loop); on an AY tone the chip's mixer for the channel (0 neither, 1 tone, 2 noise, 3 both) |
| `pitch` | Timer period offset from the note's own a frame: vibrato, slides and sweeps. For noise, an offset of the period index |
| `sample` | A DMC instrument's sample: `[address, length]` |
| `notes` | How many captured notes it was clustered from |

A song has these fields.

| Field | Meaning |
|---|---|
| `name` | Unique in the set: a model path name |
| `kind` | `music` or `sfx` |
| `origin` | `driver`, `log`, `samples`, `import` or `edited`; only files the removed `gp sound import` wrote say `import` |
| `confidence` | 0 to 1: how sure the source is these are the game's notes (docs/sound.md, "Capture") |
| `driver` | The decoder's driver, for `origin = "driver"` |
| `frames` | Its length (to the loop's end when it loops) |
| `loop_frame` | The frame the loop goes back to |
| `tempo` | `frames_per_row` and `rows_per_pattern`: the grid an editor shows. Notes keep their own frames, so the grid never changes what plays |
| `tracks` | One per channel used (below) |
| `request` | The game's request that started it (`via`, `id`, `name`), from gp gaps |
| `event` | For an effect: the model event it marks |
| `level`, `state` | For music: the level, or the rule state (`<machine>.<state>`), it plays in |
| `captured` | `recording` and `frames` (end excluded) it was taken from |
| `notes` | What capture couldn't settle |

A track is `channel`, `patterns` (each a list of notes) and `order` (the pattern played in each pattern-length column, or `null` for an empty one). The channels are the NES APU's `pulse1`, `pulse2`, `triangle`, `noise` and `dmc`, and from 0.2 an AY-8910's, by chip (`ay0` and `ay1`, the two of an SSIO board):

| Channel | Voice | What it is |
|---|---|---|
| `ay0.a`, `ay0.b`, `ay0.c` (and `ay1.*`) | `tone` | The chip's three tone channels: a square of fixed duty, the chip's clock over 16 over the period (12 bits) |
| `ay0.noise` | `ay-noise` | The chip's noise period, which its three channels share; a channel hears it through its mixer (`duty` 2 or 3) |
| `ay0.envelope` | `envelope` | The chip's envelope generator, which its three channels share; a channel follows it with `volume` 16 |

A song needn't have an AY's noise or envelope track: a register log keeps them per chip, and a song that never uses them leaves them out. The APU's player and the sound banks play only the APU's channels; an AY set plays on an arcade board.

A pattern's notes:

| Field | Meaning |
|---|---|
| `at` | Frames from the pattern's start |
| `len` | Frames it sounds |
| `pitch` | A MIDI note number on pulse, triangle and AY tone (60 is middle C; the triangle sounds an octave below a pulse with the same period); the period index 0 to 15 on noise; the rate index 0 to 15 on the DMC; the noise period 0 to 31 on an AY's noise; the envelope's shape 0 to 15 (register 13) on an AY's envelope, which the note's start writes, restarting it |
| `fine` | The timer period's offset from the pitch's nearest period, so a captured note plays at exactly the period the game used |
| `instrument` | Index into `instruments` |
| `period` | 0.2, on an AY's envelope only: the envelope's period (registers 11 and 12, 16 bits), too slow to be a pitch |
