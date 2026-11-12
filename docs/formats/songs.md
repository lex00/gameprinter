# Songs

Brought from game-printer's docs/formats/songs.md (port wave 4). Links to pages gameprinter doesn't carry yet name game-printer's docs (`~/Documents/checkouts/lex00/game-printer/docs`).

The `gp-songs` format holds songs and sound effects as notes (`gp_sound::song`). It's the one general representation every source writes and every player and exporter reads. A model's `songs` and `sound-effects` slots hold one (docs/model.md). Captured songs are game content and stay outside the repo, and tests use made-up songs.

The file is JSON. A reader takes the same major and minor version with any patch, and checks that every note's instrument exists and fits its channel.

| Field | Meaning |
|---|---|
| `format`, `version` | `"gp-songs"`, `"0.1.0"` |
| `region` | `ntsc` or `pal`: the clock periods are in |
| `instruments` | Shared by every song of the set (below) |
| `songs` | Music and effects (below) |
| `samples` | DMC sample bytes by address, when they were read: `[{addr, bytes}]` |

An instrument says what a note does frame by frame from its start; each envelope's last value holds for the rest of a longer note.

| Field | Meaning |
|---|---|
| `name` | Unique in the set |
| `voice` | `pulse`, `triangle`, `noise` or `dmc`: the kind of channel it plays on |
| `role` | `lead`, `bass`, `harmony`, `drums` or `sfx`, for sound banks and themes |
| `volume` | 0 to 15 a frame (the triangle sounds while it's above 0) |
| `duty` | Pulse duty 0 to 3 a frame, or the noise mode (1: the short loop) |
| `pitch` | Timer period offset from the note's own a frame: vibrato, slides and sweeps. For noise, an offset of the period index |
| `sample` | A DMC instrument's sample: `[address, length]` |
| `notes` | How many captured notes it was clustered from |

A song has these fields.

| Field | Meaning |
|---|---|
| `name` | Unique in the set: a model path name |
| `kind` | `music` or `sfx` |
| `origin` | `driver`, `log`, `samples`, `import` or `edited` |
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

A track is `channel` (`pulse1`, `pulse2`, `triangle`, `noise`, `dmc`), `patterns` (each a list of notes) and `order` (the pattern played in each pattern-length column, or `null` for an empty one). A pattern's notes:

| Field | Meaning |
|---|---|
| `at` | Frames from the pattern's start |
| `len` | Frames it sounds |
| `pitch` | A MIDI note number on pulse and triangle (60 is middle C; the triangle sounds an octave below a pulse with the same period); the period index 0 to 15 on noise; the rate index 0 to 15 on the DMC |
| `fine` | The timer period's offset from the pitch's nearest period, so a captured note plays at exactly the period the game used |
| `instrument` | Index into `instruments` |
