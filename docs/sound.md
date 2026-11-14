# Sound

Brought from game-printer's docs/sound.md (port wave 4; `decode`, `drivers`, `render` and `banks` in the later tools port). gameprinter has every `gp sound` command game-printer had except `export` and `import`, the FamiStudio and MIDI round trips, which were removed on 2026-11-13 since they fed tools outside the browser. No metrics are recorded. The measured results for Super Mario Bros., Mega Man 2 and Super Mario Bros. 2 are game-printer's, from games gameprinter doesn't carry. The last section is about sound on an arcade board (the AY-8910).

Music and sound effects are model data. A recording of any NES game gives them as notes and instruments; the model ties each song to a level or a rule state and each effect to an event; and one deterministic APU synthesizer plays them in every runtime. Sound is presentation, so the simulation never reads it and no state hash depends on it.

| Part | What it is | Where |
|---|---|---|
| APU register log | Every write a recording made to the APU, with its frame and CPU cycle | [apu-log format](formats/apu-log.md), `gp_sound::log` |
| APU model | Two pulses, the triangle, noise and the DMC with their length counters, envelopes, sweeps and frame counter, after the NESdev wiki's APU pages | `gp_sound::apu` |
| Capture | Log -> notes, instruments, effects, tempo, loop | `gp_sound::capture`, `gp sound capture` |
| Songs | The general song and effect representation: tracks, patterns, instruments, tempo in frames, source and confidence | [songs format](formats/songs.md), `gp_sound::song` |
| Player | Songs and effects -> register writes, a frame at a time | `gp_sound::player` |
| Synthesizer | Register writes -> 16-bit samples at 44,100 Hz | `gp_sound::synth` |
| Fidelity | A song played back against the log it came from | `gp_sound::fidelity`, `gp sound fidelity` |
| Drivers | Decoders for sound engines a pinned disassembly documents, and the registry | `gp_sound::drivers`, `gp sound decode`, `gp sound drivers` |
| Sound banks | Which instrument plays through the APU, a General MIDI program or a free SF2 or SFZ bank's patch | [sound-banks format](formats/sound-banks.md), `gp_sound::banks`, `crates/gp-sound-bank` |
| Model | Asset kinds `songs` and `sound-effects`, a slot's capture recipe, `present.sound` | [model](model.md) (core 0.4.3) |
| Themes | A restyled-audio tier: the same notes through another bank, by sound role | [theme format](formats/theme.md) (0.1.1) |
| Playback | Cues (`gp_present::sound`) for the Bevy build (WAV files) and the web preview (live, WebAudio); a hook's APU writes, live through the synthesizer, in both | game-printer's generator page, [web](web.md) |
| Agent editing | `sound_banks`, `sound_describe`, `sound_edit`, `sound_preview` | [MCP tools](mcp.md) |

## Capture

`gp sound capture --recording ID --frames A..B --name NAME [--template T] [--level L]` does the whole capture of a recording's frames. Everything it writes is game content and goes to `$GP_HOME/work/sound/<recording>/<name>/`, never the repo: `apu.log`, `songs.json`, `claims.json` (the sound requests and the channels and frames each effect took) and `fidelity.json`.

| Step | What it does | How it finds it for any game |
|---|---|---|
| 1. The log | The recording's APU writes ($4000 to $4013, $4015, $4017) with each write's frame and CPU cycle within it | From the recording's event stream when it has one, else a replay with gp-nes's event log on. Observe-only: the replay changes nothing, so BEHAVIOUR_VERSION stays 6 |
| 2. Frame states | The log applied to the APU model, each channel's period, volume, duty and trigger (a write to its fourth register) read at the end of every frame | The APU model, the same for every game |
| 3. Notes | A note starts when a channel begins to sound, or when it's triggered and its pitch moved by a quarter tone or its volume rose (the envelope restarted). Changes without a trigger stay in the note as envelopes: volume, duty and period offset per frame, which keeps vibrato, slides and hardware sweeps | Register behaviour, not driver data |
| 4. Instruments | Notes on the same kind of channel whose envelopes agree over the whole note (volume within 1 on 90% of frames, duty and pitch offsets equal, the instrument's last values held) are one instrument, whose envelopes are its longest note's | Clustering, no names needed |
| 5. Music and effects | Each sound request gp gaps finds (a queue byte or a play-sound routine, named by the disassembly's constants; [gaps](gaps.md)) is music when its interface or name says music or song, else an effect. An effect claims the channels that start a note within 3 frames of it (only the one its interface names, such as `Square1SoundQueue`), until the channel falls silent, its duty returns to the music's, the next request on it, or 120 frames. Music is cut at each music request and keeps what effects didn't claim | gp gaps' detector, run over the whole recording since an interface is known by how many places use it |
| 6. Tempo and loop | Frames per row: the largest count up to 32 that divides 90% of the note starts' distances from the first. Loop: the smallest period of 60 frames or more after which every unclaimed channel repeats to the capture's end, for at least one full period. Rows per pattern: the first of 16, 32, 12, 24, 8, 64, 48 that puts the loop and the end on pattern boundaries | Timing alone |
| 7. Roles | Effects' instruments are `sfx`; in music noise and DMC are `drums`, the triangle `bass`, the higher pulse `lead` and the other `harmony` | Channel and pitch |
| 8. Bindings | Effects bind to a model event by the words of their names (`Sfx_EnemyStomp` -> `enemy.stomped`, through a small table of general synonyms such as coin -> collected); music to a rule state the same way (`EndOfLevelMusic` -> `game.level_end`); `--level` goes to the music that plays while the player is in control most (the state mapping's start condition) | Names and the state mapping; a binding no word supports stays empty, and the song isn't ready to edit until a person or an agent binds it |

Each song records where it came from (`origin`: `driver`, `log`, `import`, `edited`; `import` is read only in files the removed `gp sound import` wrote; `samples` is reserved for capture from audio, not built) and a `confidence`: for log capture, the share of the song's channel frames no effect took, lowered by a fifth when no loop was found; for an effect, 0.9 when its interface names its channel, 0.7 otherwise, lowered when it ran into the 120-frame cap.

### Results

On the store's recordings, frames 1 to 2,400 of each (the warpless run's 1-1 for Super Mario Bros., the Wood Man and 1-1 baselines for the other two). Agreement is each captured song played by the player and synthesizer against the log's own frames over the same span, with effects' channels left out of music and only an effect's channels compared for an effect: onset F1 (same channel, same pitch within a quarter tone, within a frame), the share of sounding frames with the same period and with the volume within 1, and the Pearson correlation of the two renders' loudness per frame.

| Game | Captured | Music onset F1, pitch, volume | Effects onset F1 | Loudness (music) | Notes |
|---|---|---|---|---|---|
| Super Mario Bros. 1-1 | 6 songs (the overworld theme twice around the bonus room, the underground theme, end of level, two short event songs), 6 effects, 24 instruments | 1.0, 1.0, 1.0 | 1.0 | 0.69 | The capture is shorter than one pass of the ground theme, so no loop is found (confidence 72%); the driver decoder supplies it |
| Mega Man 2, Wood Man | 1 song, 9 effects, 63 instruments | 1.0, 1.0, 1.0 | 0.84 | 0.90 | Mega Man 2 asks for music and effects through one routine (`sound_queue_push`), so the stage select and stage music are one song; filed |
| Super Mario Bros. 2, 1-1 | 3 songs, 3 effects, 17 instruments | 1.0, 1.0, 1.0 | 1.0 | 0.93 | |

The test program `test-roms/apu/notes.s` (ours, a made-up phrase) captures to one song of 96 frames with its loop at 0, 12 frames a row, 18 notes and 3 instruments, with every agreement number at 1.0 (`crates/gp-cli/tests/detectors.rs`, `the_notes_test_rom_captures_and_holds_to_its_log`, in `just check`).

Loudness correlates below the notes because the player writes constant-volume registers where a game used the hardware envelope and resets a pulse's phase at other moments; the levels match frame by frame, the waveforms' phases don't.

## Drivers

`gp sound drivers` lists the registry (`gp_sound::drivers`). A driver is a sound engine several games share; the registry finds it by the asset map format its music data lifts as, so a new game whose map lifts that format reuses the decoder with no code. An arcade driver whose music data isn't mapped has the format `ay-log`: its decoder reads the songs from the AY-8910 register log the sound program wrote, so it works on any game the board runs, whatever the romset.

| Driver | Format | Decoder | Games |
|---|---|---|---|
| `nintendo-dk` | `dk-music` | No: no asset map lifts Donkey Kong's music data yet. `gp sound capture` reads its songs from the APU log, and the model keeps the driver as state (`dk_sound`, below) | `donkey-kong-ju`, `donkey-kong-oe` |
| `midway-ssio` | `ay-log` | Yes, from an AY register log: each run of frames where a channel sounds, ended by `--gap` frames (60 by default) of silence on all six channels, is a song, holding each channel's notes as `gp sound capture` makes them | `tapper`, `rbtapper`, `tapperg`, `sutapper`, `tappera` |

`gp sound decode --source SOURCE` finds the source's asset map (`--maps DIR`, else `$GP_ASSETS`, else the tool's `assets/` and each game's `assets/` in the games checkout), and the driver by an asset's format or, failing that, by the games the registry lists. A driver with no decoder is refused in words. On Tapper, `--ay-log` names `gp sound capture`'s `ay.log` (or its folder):

```sh
gp sound capture --recording c0130f81267d --frames 1..3000 --name five
gp sound decode --source tapper --ay-log $GP_HOME/work/sound/c0130f81267d/five --against $GP_HOME/work/sound/c0130f81267d/five/songs.json
```

`--song name=FIRST-LAST[@LOOP]` names a song by its frames instead (LOOP counted from FIRST). The songs go to `$GP_HOME/work/sound/decoded/SOURCE/songs.json` (`gp-ay-songs 0`, a `songs` list, each with its name, first and last frame, loop and channels). `--against` holds each decoded song to a capture's notes: the note starts (channel, frame, period) that match at the frame offset where most do, as an F1. On frames 1 to 2,999 of the five-level Tapper recording (c0130f81267d): three songs, the long one (frames 1,102 to 2,837) matching 794 of the capture's 935 notes, F1 0.918.

game-printer's registry held Super Mario Bros.' decoder (`smb`, from SMBDIS.ASM's music routines, with an onset F1 of 0.895 on the ground theme against its capture) and two drivers without decoders, Mega Man 2's and Super Mario Bros. 2's. They were cut with those games.

## Playback

`gp_present::sound` compiles `present.sound` into cues, written to `assets/sound.txt` as songs by level index and by rule state and effects by event number, and its `Director` reads the world and the frame's events after each tick and says which song should play and which effects start: a rule state's song over the level's. It reads the world and never writes it.

| Runtime | How it plays |
|---|---|
| Bevy (`gp gen bevy`) | The generator renders each bound song (one pass and one loop) and effect into `assets/sound/<slot>/<name>.wav` through the APU synthesizer, or through a theme's sound bank when its restyled-audio layer maps the slot's role. The render crate's `sound` module, built with the `audio` feature, plays the level's song on loop and each effect once with Bevy's audio. `play` has the feature by default; the headless builds and the Linux container build without it, since it needs the platform's audio libraries (game-printer's generator page) |
| A theme's sound per role | The theme's file for each sound role, rendered to `assets/sound/roles/<role>.wav`, played on the role's cue by `gp_present::sound::RolePlayer` in both runtimes, in place of the model's own sound ([theme format](formats/theme.md#sound-replacements)) |
| Web preview | `gp-web-prep` writes `assets/sound.json` (the cues, the song sets, and the roles a theme replaces); the wasm module runs the same `Director`, `gp_sound::player` and `gp_sound::synth` after every step (`gp_load_sound`, `gp_audio`), and `preview.js` queues each frame's 735 samples as a WebAudio buffer once a key is pressed |

The player turns notes into register writes the way a simple driver does: constant volume from the instrument's envelope each frame, the period plus the pitch envelope, the high period byte only when it changes (a write there restarts a pulse's phase), the triangle held by its linear counter, noise retriggered per note, DMC samples started through $4015. An effect takes the channels it uses until it ends, as NES drivers give effects priority.

### A game's own driver, kept as state

A model can keep a game's sound driver as state instead of binding songs: its hook steps the driver's bytes once a frame, as the game's NMI does (in `frame_start`, with core `entity.hook_frame_start`, so the step comes first in the frame as the NMI's does), and the bytes are counters the state mapping compares. The hook's `apu` method (`gp_sim::Ext::apu`, default none) gives the register writes that step makes, worked out on a copy of the world before the frame, so presentation never changes the simulation. `gp_sim::ext::hook_apu` collects every hook's writes in body order.

| Where | What plays them |
|---|---|
| Web preview | `Preview::step` takes the hooks' writes before each tick and the synthesizer plays them in place of the cues' player from the first frame that has any. `gp-web-prep` writes an empty `assets/sound.json` for a model with hooks that binds no sound, so the page turns audio on |
| `gp interp run --apu-log FILE` | The writes as an untimed APU log, every write at cycle 0 of its frame; `gp sound render FILE --out FILE.wav` plays it |
| Bevy | The render crate's `sound` module, with the `audio` feature: `game_sim::ApuWrites` holds each tick's writes (worked out in the tick's first system, on the world before it, as the interpreter does), and from the first tick with any the synthesizer makes its 735 samples, streamed to one Bevy audio player (`ApuStream`, a source that never ends and plays silence while nothing waits; at most a fifth of a second waits, older samples are dropped). The cues' music stops then, as in the web preview. A muted channel plays at gain 0 |
| `gp gen conform` | `headless --apu-hashes FILE` writes each tick's writes' BLAKE3 (register, value, in order) a line a tick, and the suite compares them with the interpreter's `hook_apu` on every frame of every input, as it does the state hashes. A run whose states agree and whose writes don't says from which frame |

Donkey Kong's `dk_sound` is one (`ext/src/sound.rs` in the `donkey-kong` package). It is stepped from the original's driver bytes at each NMI. Its writes equal the driver's writes in the recording's APU log, in order. They do on 7190 of 7190 frames of the user's play (recording ad8352d8c24f) and 5220 of 5220 of run 5658S (76c9bb11a914). Its `apu` also gives the game's own `$4015` writes that change what sounds: the channels turned off while no song plays on the title and in the attract demo ($C8F8, $CE85), which keep the demo silent.

Donkey Kong's `dk_sound` steps the driver in `frame_start` (core `entity.hook_frame_start`, docs/model.md), at the start of every frame before the first system, frozen or not, as the game's NMI runs it first ($C890 calls $FA48). There it takes the requests the last frame left, from the rules and the hooks alike. The driver plays through the Start pause. The hook's `step` runs in the frame's body order and keeps only the death's sound and its wait ($3A).

The model asks for every request 25m makes, and the state mapping compares the four request bytes ($FC to $FF):

| Request | Who asks, from the game's code |
|---|---|
| $FF, pulse 1 effects | Mario's walk, climb and jump (dk_mario), the smash (smash_test), the death (dk_sound) |
| $FE, pulse 2 and triangle | Kong's beats (dk_throw, $EB85 and $EB8E, reading the throw timer and the wait $44 as the last frame left them, since $EB06 runs before the timers at $CF27), the death's first count (dk_sound) |
| $FD, music | The rules: song 1 at a new game ($CA1B), 2 at the clear ($D084), 3 at the stage setup ($CB44), 6 at the pause and the unpause ($CF8C); dk_sound: the title's song 7 when the title's routine runs with $0518 at 0 ($C8FE, the counter `title_song`, which each attract demo takes one from at $CA26) |
| $FC, the jingle | The rules and dk_hammer: 25m's ($02) as the stage starts ($CCA3) and as the hammer is dropped ($D7EC), the hammer's ($40) as it is taken ($D63F), none through a death ($D0C0), the clear ($D088) or the pause, which keeps it in $0F and asks for it again at the unpause ($CF7F, $CF6C) |

Effect 8, Mario's walk, picks one of two pitches by bit 3 of the random byte $18 ($FC62). The model doesn't hold $18 yet (docs/cycles.md), so it plays the lower pitch; in a lockstep with the original's draws it takes the byte from the soft draw site `sound_buzz` (a state mapping's `[[draws]]` with `soft = true`, docs/formats/draws.md). Only the sound reads that site, and it never puts the draws out of step.

### Holding the sound to the original's

`gp interp compare` and `gp clip score` measure the sound of a model whose hooks write the APU: on each compared frame, the original's APU writes (gp-nes's event log, its lag frames' included) against the model's (`gp_sim::ext::hook_apu_peek`, worked out before the step), by `gp_sound::compare`, for any game. Two measures: the frames whose writes are the same registers and values in order, and the frames whose every channel's period, volume, duty and restart are the same at the frame's end, each side's writes played through the APU model. Both sides start with the four tone channels on, since a run from the middle of a game doesn't know what the APU held. `gp sound compare ORIGINAL.log MODEL.log --offset N` does the same for two register logs.

Donkey Kong, measured straight through (free mode, with the original's draws) on 2026-11-03:

| Run | Frames | Writes the same | Every channel the same |
|---|---|---|---|
| The user's play (recording ad8352d8c24f), from frame 1200 | 6017 | 6011 | 6017 |
| The 121 clips of the mechanic sessions with `gp clip score --all` | 77927 | 76342 | 77646 |

Every channel agrees on every frame of 117 of the 121 clips, and of all 109 whose mapped state agrees on every frame straight through. The other 4 (climb-ladder 008 to 010 and hammer 030) are clips whose mapped state splits straight through too. On the user's play the 6 frames whose writes differ hold writes that change no channel, the first the model's `$4015 = $0F` on its first frame.

### Hearing Super Mario Bros.' 1-1 music

The model binds the overworld song to `w1_1`, the end-of-level song to `game.level_end`, and the stomp and coin effects to `enemy.stomped` and `item.collected` (`templates/smb1-ntsc-prg0/model.toml`, `[present.sound]`; the slots name their capture recipe, recording `782c557d` frames 1 to 2,400). With the ROM in `$GP_ROM_DIR` and the store's recordings:

```sh
gp sound capture --recording 782c557d --frames 1..2400 --name smb-1-1 --level w1_1 --template smb1-ntsc-prg0
gp gen bevy templates/smb1-ntsc-prg0 -o $GP_HOME/work/gen/smb1 --assets $GP_HOME/work/assets/smb1-ntsc-prg0
cd $GP_HOME/work/gen/smb1 && cargo run --locked -p game --bin play
```

and for the web preview, `scripts/web/build-preview.sh OUT templates/smb1-ntsc-prg0 $GP_HOME/work/assets/smb1-ntsc-prg0` builds the page with `assets/sound.json` (gp-web-prep reads the capture under `$GP_HOME`); serve OUT on localhost ([web](web.md)), open `index.html` and press a key to start the sound. `gp sound render <capture>/songs.json --song areamusicqueue_0x01 --out 1-1.wav` writes the song alone.

## Sound banks and themes

A sound bank mapping ([format](formats/sound-banks.md)) gives each instrument, or every instrument of a role, a target: the APU (the default), a General MIDI program, or a bank's patch. `gp sound banks` lists the banks found in `$GP_SOUND_BANKS` and `$GP_HOME/sound-banks`; none ship with Game Printer, and a bank's licence is read from a `.license` or `LICENSE` file beside it or its INFO chunk. `crates/gp-sound-bank` renders SF2 banks with rustysynth (MIT, pure Rust): banked channels as MIDI events, the rest on the APU, mixed. SFZ files are listed and mapped but not rendered yet (filed); their channels fall back to the APU and the render says so.

A theme's restyled-audio tier ([theme format](formats/theme.md), 0.1.1) names a mapping for the `sound` group or a role (`music`, `music.level`, `sfx.game`), so a theme remasters the music without touching the notes or the simulation. The Bevy generator renders the WAV files through it.

## Editing by asking an agent

There's no music editor in Game Printer. A person asks an agent ("make the bass a real bass guitar", "1-1 a little slower") and the agent uses the MCP tools ([mcp](mcp.md)):

| Tool | What it does |
|---|---|
| `sound_banks` | The banks and their patches, each with a one-second preview WAV |
| `sound_describe` | A working copy's songs: channels, instruments with roles and what each plays through, origin and confidence, tempo and loop, bindings, and whether each is ready to edit |
| `sound_edit` | Deterministic ops (`gp_sound::edit`): remap, role, transpose, tempo, mute, solo, gain, swap. Checked against the NES's limits (nothing is clamped), written to the copy's `sound/<slot>.songs.json` and `sound/banks.toml` (or a theme's layer), with before and after WAV clips, history and undo. |
| `sound_preview` | The copy run in the interpreter on a baseline's input with its music and effects cued as the game cues them, into a WAV |

A song is ready to edit when the model knows it completely (`gp_sound::edit::readiness`). That means at least 80% confidence in the capture of its channels and a role for every instrument. Its loop must be known, and it must be bound to a level or rule state (an effect to an event). An op that names a song that isn't ready is refused with what's missing. Super Mario Bros.' 1-1 capture isn't ready (72%, no loop): capture over a longer run, or swap in the driver-decoded song.

## Not yet

These were rows in game-printer's open questions: the Mega Man 2 and Super Mario Bros. 2 driver decoders; music and effects through one play-sound routine; DMC sample bytes in capture (the event stream has the writes, not the bytes the DMC reads); SFZ rendering; capture from rendered audio (`origin = "samples"`); and live mixing of effects over music in the Bevy build, which plays bound songs as pre-rendered files (a hook's APU writes play live).

## On an arcade recording (AY-8910)

`gp sound capture --recording ID` on an arcade recording (port wave 6) writes the register log of Tapper's two AY-8910s, `ay.log`, from the recording's `ay-write` events (or a replay with the event log on), and `songs.json`, each of the six channels' notes: runs of frames with one tone period while the mixer lets the tone through and the volume isn't 0, with the pitch in hertz (the chips' 2 MHz clock over 16 over the period) and the volume of each frame. `gp sound fidelity DIR` holds the notes to the log frame by frame (onsets, pitch, volume); `gp sound compare A/ay.log B/ay.log` holds one log to another (frames with the same writes, frames whose every channel's state is the same; exit 1 at the first that differs). The format is in `crates/gp-cli/src/sound_ay.rs`. `--template` (songs bound to a model through gp gaps' sound requests) and the renders' loudness are the NES's and not here for the AY; `gp sound render` plays the log through milestone 3's AY synth (`gp_arcade::sound`), below.

On Tapper (a 1,282-frame clip): 2,114 AY writes, notes by channel 0a 93, 0b 32, 0c 25, 1a 36, 1b 28, 1c 26; fidelity 240 of 240 onsets, pitch and volume on all 3,046 sounding channel frames. The same clip with the first tap 10 frames earlier: writes the same on 943 frames, channel states on 910, first differing at frame 865. The log's writes are, frame by frame, milestone 3's own AY write list (held to MAME's), on 900 frames (a test, skipped without tapper.zip).

### Rendering and the decoder on AY

`gp sound render` takes an `ay.log` or an AY song file (`gp-ay-songs 0`, from a capture or `gp sound decode`) as well, and plays it through milestone 3's board sound (`gp_arcade::sound`: the two AYs, the SSIO's duty-cycle volumes and its mute, MAME's resampling) into a stereo WAV at 48 kHz, AY 0 left and AY 1 right. The log keeps each frame's writes but not their times inside it, so every write lands at its frame's start, 30 frames a second. The SSIO's PROM sets the voices' loudness; `--rom` names the romset, else tapper's is looked for in `$GP_ROM_DIR`, and without one every voice plays at full volume. A song renders from the register writes its notes give (`--song`, `--loops` back to its `loop_frame`). `--banks` is the NES's and refused.

On the five-level Tapper recording, the decoded song of frames 1,102 to 2,837 renders to 57.9 seconds. The code is `crates/gp-cli/src/sound_ay_render.rs`.
