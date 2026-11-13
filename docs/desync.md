# When an imported movie desyncs

A movie imported with `gp movie import` ([movies.md](movies.md)) replays exactly in `gp-nes`: the recording is what our runtime does with the movie's buttons. It desyncs when that isn't what the author's emulator did, and the run then goes somewhere the author never went. This is the procedure for finding out why, in order. Each step has its command. Work in a scratch store (`export GP_STORE=$(mktemp -d)/store`, which keeps `$GP_ROM_DIR` where it is) until the last step; only a movie that syncs goes into the real store.

Brought from game-printer's docs/desync.md. Its steps are the same. Three helpers it used are game-printer's `scripts/movie/` and aren't in gameprinter: `ram_drift.py` (step 3), `sweep.sh` with `variants.py` (step 4) and `fcm2fm2.py`, which converted an old FCEU `.fcm` movie to `.fm2`. Each step says what to run instead. The worked examples on Mega Man 2, Kirby's Adventure, Ninja Gaiden and Super Mario Bros. 2 stay in game-printer, with a summary at the end; Donkey Kong's is in [corpus.md](corpus.md).

## 1. Read the movie's header and notes

Compare what the movie says about its emulator with what we emulate, before running anything.

| Header field | What to check |
|---|---|
| `.fm2` `emuVersion`, the submission or publication page | FCEUX version; TASVideos pages also name the emulator |
| `.fm2` `NewPPU` | `0` is FCEUX's old PPU, whose timing isn't a real console's. TASVideos has asked for the new PPU since FCEUX 2.2; an old-PPU movie may depend on timing no accurate emulator has |
| `.fm2` `RAMInitOption`, `RAMInitSeed` | FCEUX's power-on RAM. Option 0 (the default) fills internal RAM with `00 00 00 00 FF FF FF FF` repeated (FCEUX 2.6.6 `src/fceu.cpp`, `FCEU_MemoryRand`); `gp-nes` powers on with zeros, or ones with `--ram-init ones`, or FCEUX's pattern with `--ram-init fceux` |
| `.fm2` `palFlag`, `port0` to `port2`, `fourscore`, `FDS` | Refused by the import when unsupported ([movies.md](movies.md#what-is-imported)) |
| `.fm2` frame commands, BizHawk `Reset` and `Power` | Resets are kept (input log format 1). Where FCEUX lands one is in [movies.md](movies.md#what-is-imported) |
| `.bk2` `Core`, `SyncSettings` | NesHawk or QuickNES, region, power-on RAM, which BizHawk keeps in the movie's sync settings |

A game that reads RAM before writing it (an RNG seed, a frame counter) depends on the power-on fill, so a movie made with another fill can desync from the first frame. `gp movie import --ram-init fceux` and `gp record --ram-init fceux` power on with FCEUX's fill.

The iNES header can matter too. Some dumps carry text in bytes 10 to 15, which a few loaders read as unofficial fields (region, PRG RAM, bus conflicts). `gp roms scan --json` shows what is around a dump's game data, and `gp roms clean` writes a copy with the header from the game's roms.toml. On 2026-10-27 game-printer compared the first 3,000 frame hashes of two dumps with their headers as they were and with bytes 10 to 15 zeroed, and they were identical: `tetanes-core` reads bytes 8 to 15 only for NES 2.0.

FCEUX clears battery RAM for a movie (`FCEU_ClearGameSave`, called from `PowerNES` when a movie loads) and powers MMC3's PRG RAM on as zeros (`GenMMC3Power`), which matches `gp-nes`.

## 2. Find the first wasted input

```sh
gp movie import MOVIE --source ID --record --name try
gp movie diagnose <recording>
```

`gp movie diagnose` reads the recording's input log, whose polled flags are the frames the game really read, and lists input changes the game never saw: buttons changed while the game wasn't reading and gone by its next read. It also lists changes inside a lag run. A TAS wastes no input, so the first wasted input is the earliest point where our replay has left the author's path, often long before the visible death. Read nothing past it.

The tests in `crates/gp-cli/src/diagnose.rs`, which `just check` runs, show the rule on small logs: a press released before the next read is wasted, and one held into it isn't.

No wasted input up to the symptom means the input lines up and the game state itself differs: go to step 3.

## 3. Name the byte that drifts

Record two variants that end differently (from step 4, or zeros against ones), then compare their RAM:

```sh
gp trace diff A B                                  # the first frame and fields that differ
gp query A --frame 600 'ram[*]' --json > a-600.json
gp query B --frame 600 'ram[*]' --json > b-600.json
gp labels lookup GAME '$004A'                      # what a game's labels call a byte
```

A byte that differs from power-on on and never converges is state the game carries from uninitialized RAM; one that starts differing later points at the slip. Compare the two reads at a few frames (200, 600, 1,500, 3,000) and keep the bytes that differ at every one.

Not here yet: game-printer's `scripts/movie/ram_drift.py A B --frames ... --labels FILE`, which read internal RAM from both traces at each frame and listed the bytes that differ, those that differ every time first, named from a disassembly's labels.

## 4. Sweep alignments and power-on fills

Record a variant for each shift and fill into its own scratch store, and print each one's wasted inputs and the first wasted frame:

```sh
for shift in -2 -1 0 1 2 3 4; do
  for fill in zeros ones fceux; do
    export GP_STORE=$(mktemp -d)/store
    id=$(gp movie import MOVIE --source ID --shift $shift --ram-init $fill --record --json | jq -r .recording.id)
    echo "shift $shift, $fill: $GP_STORE $id $(gp movie diagnose $id --json | jq -c '[.wasted_count, .wasted[0].frame]')"
  done
done
```

Read how far each one got, with `GP_STORE` set to its store, with `gp run-until` on the game's stage byte (Donkey Kong's is `ram[0x0053]`), or `gp coverage` for a game with a coverage definition. The best variant reaches the most levels with the fewest wasted inputs; a shift is only right if it removes wasted inputs from the start. It never writes the real store.

Not here yet: game-printer's `scripts/movie/sweep.sh MOVIE ID --shifts -4..6 --ram zeros,ones [--reset-shifts -2..2] [--frames 12000]`, which did this in one command and also moved each soft reset by a few lines (`variants.py`), and cut variants short with `--frames`. `--shift` moves every input and a reset with it; there is no flag yet that moves a reset alone.

## 5. Check our emulator against another

If another accurate emulator plays the input log to the same states, `gp-nes` isn't the cause, and the difference is between the author's emulator and accurate ones. If it goes further, the frame where `gp-nes` leaves it is a fork bug to triage.

The Mesen oracle (`gp oracle record <recording>`, `gp oracle diff`, [oracle.md](oracle.md)) plays a recording's input log in Mesen 2 and finds where the two part. For an arcade set, the MAME oracle plays an input log in MAME and writes every frame ([oracle-mame.md](oracle-mame.md)).

## 6. Record the outcome

A movie that syncs is imported into the real store, and the exact command goes into [corpus.md](corpus.md) under its game. One that doesn't gets a dated line in [corpus.md](corpus.md) with the evidence: frame numbers, the variants tried, what drifted.

## Worked examples

Donkey Kong's runs are in [corpus.md](corpus.md): 1352M desyncs because the game steps its random numbers on every pass of its wait-for-frame loop, so the barrels choose by the emulator's CPU and PPU timing, and 5658S syncs into 100m.

game-printer's docs/desync.md worked through four more games, which aren't in gameprinter. What each found:

| Movie | What the steps found |
|---|---|
| Mega Man 2 submissions 10597S, 10604S, 10636S (FCEUX 2.6.6, old PPU) | The game seeds its RNG and frame counter from uninitialized RAM, and it boots one frame later in `gp-nes` than in FCEUX. With `--shift 1 --ram-init fceux` two runs reach all 14 stages. The wasted-input count didn't single out the right variant: the authors pressed buttons during boot and transitions, which only an optimized TAS doesn't do |
| Kirby's Adventure 3226M (FCEUX 2.2.2) | A soft reset pressed before its own line's frame loses Kirby's lives in 1-2; pressed one frame later, as FCEUX does, the run reaches 4-2. That is why the import presses an `.fm2` reset before the next line's frame |
| Ninja Gaiden 4751M (BizHawk 2.8) | No wasted input at shifts 0 and 3 to 5; at `--shift 4` act 1 plays through and the lives are lost at its boss, the same in Mesen, so the difference lies between NesHawk and the accurate emulators |
| Super Mario Bros. 2 6366M (BizHawk 2.9.1) | Synced as imported, with 3 wasted inputs, all on menus. It named its ROM by `MD5`, which is why a `.bk2` may |
