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

Recordings read gp-nes's state: an arcade game has no recording format yet (wave 6 of the port).
