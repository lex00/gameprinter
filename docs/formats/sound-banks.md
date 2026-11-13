# Sound bank mappings

Brought from game-printer's docs/formats/sound-banks.md with gp sound render, export, import and banks. `gp sound export` and `import` were removed on 2026-11-13.

A `gp-sound-banks` mapping says what each captured instrument sounds like (`gp_sound::banks`; this page is version 0.1.0), and every instrument plays on the faithful APU synthesizer unless a rule sends it elsewhere: to a General MIDI program (for GM banks) or to a free sound bank's patch, an SF2 SoundFont preset or an SFZ file's regions. A working copy keeps its mapping in `sound/banks.toml`; a theme's restyled-audio layer names one ([theme format](theme.md)); `gp sound render --banks` takes one. The format is TOML, in the same style as the image formats' configs; a reader takes 0.1 with any patch.

```toml
format = "gp-sound-banks"
version = "0.1.0"

[[bank]]
name = "free"
kind = "sf2"
path = "free.sf2"
license = "CC0-1.0"

[[map]]
role = "lead"
target = { kind = "bank", bank = "free", preset = "Square Lead" }

[[map]]
instrument = "triangle-1"
target = { kind = "gm", program = 33 }
gain = 0.8

[mix]
mute = ["noise"]
gain = { pulse2 = 0.5 }
role_gain = { drums = 0.7 }
```

| Item | Fields | Meaning |
|---|---|---|
| `[[bank]]` | `name`, `kind` (`apu`, `sf2`, `sfz`), `path`, `license` | A bank a rule can name. `path` is absolute or relative to a bank folder: `$GP_SOUND_BANKS` (paths split as PATH splits), then `$GP_HOME/sound-banks`, and for a working copy its `sound/` folder |
| `[[map]]` | `instrument`, `role`, `song` (at least one), `target`, `gain`, `transpose` | A rule. For an instrument in a song, the first rule naming the instrument wins, then the first naming its role, then the first naming neither; `song` limits a rule to one song. `transpose` is semitones when played through a bank |
| `target` | `{ kind = "apu" }`, `{ kind = "gm", program }` (0 to 127), `{ kind = "bank", bank, preset or program, bank_number }` | What the instrument plays through. An SF2 preset is found by name or by bank and program |
| `[mix]` | `gain` (by channel), `mute`, `solo` (channel lists), `role_gain` | Channel gains for the synthesizer; a solo mutes every other channel |

Roles are `lead`, `bass`, `harmony`, `drums` and `sfx`. A mapping is refused when a rule names a bank it doesn't list or a role outside these. So is a channel that isn't one of the five, or a program past 127.

`gp sound banks` (and the MCP tool `sound_banks`) lists what's found: the APU's four voices, each SF2's presets (from its `phdr` chunk) and each SFZ's regions with their key ranges, with the licence from a `<file>.license`, `LICENSE` or `LICENSE.txt` beside it, or the SF2's copyright and name. No bank ships with gameprinter: add free ones with their licences.

`crates/gp-sound-bank` renders a mapping. A channel plays through a bank when most of its notes' instruments map to the same bank target. It goes to rustysynth (MIT) as MIDI events, a note on at each note's first frame with the velocity from its first volume and a note off after its last, noise and DMC on the percussion channel by the drum map; the other channels play on the APU with the mix's gains, and the two are summed. SFZ banks aren't rendered yet: their channels play on the APU and the render lists them as fallbacks.
