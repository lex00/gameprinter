# The Mesen oracle

Mesen 2 is the reference emulator `gp-nes` is checked against (game-printer's Milestone 1, step 5; brought to gameprinter on 2026-11-13). An arcade romset is checked against MAME the same way, through the same commands ([On an arcade romset](#on-an-arcade-romset)). `gp oracle record` runs a recording's input log from power-on in Mesen and stores what Mesen did as a recording of its own. `gp oracle diff` compares the two field by field and frame by frame. It classifies each divergence and then resyncs to find the next one. Each class gets a verdict in `oracle/triage.toml` (the verdicts are under Triage below), and a recording whose diff is fully triaged with no fork bug can be marked golden.

Mesen is GPL-3.0. It runs only as an external program in its own Docker image, built from source; nothing in the repo links, vendors or commits it. Our capture script, `scripts/oracle/capture.lua`, is ours and runs inside it.

## Running it

| Step | Command |
|---|---|
| Build the Mesen image (once per architecture) | `just mesen-build` or `just mesen-build linux/amd64` |
| Record a run in `gp-nes` | `gp record --source donkey-kong-oe --input run.input` |
| Run the same input in Mesen | `gp oracle record <id>`, with `--platform linux/amd64` for the other architecture |
| Compare them | `gp oracle diff <gp-nes id> <mesen id>` |
| Triage a class | `gp oracle triage <class> <verdict> --note "<evidence>"` |
| Mark the `gp-nes` recording golden | `gp oracle golden <gp-nes id> --against <mesen id>` |
| All of it for the baselines, on both architectures | `just check-mesen` |

`just check-mesen` builds both images and then runs `gp oracle check` on the gp-testrom baseline. Each baseline is recorded in `gp-nes` and in Mesen on linux/arm64 and linux/amd64, in a temporary store, so nothing is written to yours. The check fails (exit 1) if the two architectures' Mesen recordings differ, or if the diff finds a divergence no triage record covers. Known fork bugs are listed without failing it.

`gp oracle record` and `gp oracle diff` work on any recording in the store, not only baselines. The diff writes its report into the store as a JSON object and prints its hash. It exits 1 when it finds any class, 0 when the two agree on every compared field of every frame, and 2 when it can't run.

## How Mesen runs headless

The spike of 2026-10-27 tried Mesen 2's own test runner mode first. It worked, so neither the `mesen-agent` fork nor the GUI build under a virtual display was needed. `Mesen --testRunner script.lua game.nes` loads the ROM and the script without creating a window, runs at maximum speed, and exits with the code the script passes to `emu.stop()` (UI/Utilities/TestRunner.cs). With `--debug.scriptWindow.allowIoOsAccess=true` the script can read and write files.

| | |
|---|---|
| Source | `SourMesen/Mesen2` at `b9fa69ddc6d0a331fb103fdb5eef6904305703c2`, its last commit: the repository is archived at version 2.1.1 and work continues as `nesdev-org/MesenCE`. git checks the commit hash, so the source is pinned by content |
| Image | `docker/mesen/Dockerfile`, tagged `game-printer-mesen:b9fa69dd-arm64` or `-amd64`, separate from the toolchain image. The .NET 8 SDK and runtime base images are pinned by digest. clang, make, zip, git, ca-certificates and SDL2 are pinned by version and installed from snapshot.debian.org at the timestamp both base images were made from (`DEBIAN_SNAPSHOT`, 2026-10-20). The NuGet packages are restored in locked mode from `docker/mesen/nuget/`, one lock file per architecture with every package's content hash, so a package that changed fails the build. The one floating NuGet reference, `Dotnet.Bundle` (for macOS app bundles), is pinned to 0.9.13. The image is the .NET 8 runtime plus SDL2 and the built program |
| Build time | 140 s on linux/arm64 (native, Apple M-series, 18 cores) and 224 s on linux/amd64 (QEMU emulation), from a cold cache, 2026-10-27. `just mesen-build` records it as `docker.mesen.build.<arch>.duration` |
| Run time | 20 s for gp-testrom's 3,600 frames on linux/arm64 and 34 s on linux/amd64 under emulation (game-printer, 2026-10-27); 7 s for Donkey Kong's 900 on linux/arm64. `gp oracle check` records them as `oracle.check.<baseline>.<arch>.seconds` |
| Settings | A `settings.json` of `{}` next to the program puts Mesen in portable mode and skips its first-run wizard, which would otherwise open a window. Everything else is set by switches on each run. Each run gets a fresh work directory, and the container runs with no network as the calling user (the image makes its program folder writable by any user for that) |
| ROM | The image `gp-nes` boots, `gp_nes::board::runnable`'s (docs/sources.md), written into the work directory and mounted read-only, not the file as found in `$GP_ROM_DIR`. So a junk or nonstandard header can't make the two emulators boot differently |

`gp oracle record` sets Mesen up from the recording's `power_on` ([formats/recording.md](formats/recording.md)) with these switches.

| `power_on` | Mesen switch | Notes |
|---|---|---|
| `region` | `--nes.region=Ntsc` or `Pal` | |
| `ram_init` | `--nes.ramPowerOnState=AllZeros` or `AllOnes` | `random` is refused: Mesen's random RAM can't be seeded |
| `cpu_ppu_offset` | `--nes.randomizeCpuPpuAlignment=false` | Only 1 runs; Mesen's alignment is fixed or random. It still differs from `tetanes-core`'s (Results below) |
| `ppu_warmup` | `--nes.restrictPpuAccessOnFirstFrame` | |
| `mapper_revisions` | none | Only the defaults run until a board that needs them is compared |
| `four_player`, `zapper` | `--nes.port1.type=NesController`, `--nes.port2.type=NesController` | Only standard controllers. The wizard is what normally plugs them in, so without these switches no controller is connected |
| `concurrent_dpad` | `--nes.allowInvalidInput` | |
| `render_video`, `mix_audio` | none | Mesen always renders and mixes, so a headless recording's Mesen run differs here; `gp oracle record` prints a note |

`--nes.randomizeMapperPowerOnState=false` is passed too.

## The capture

The script plays the input log and writes one record per frame. A record holds the values of Mesen's `emu.getState()` the comparison can use, and Mesen's memories (internal RAM, OAM and the rest) read without side effects. By default it also logs events: interrupts, controller reads, and register writes.

| What | How the script does it |
|---|---|
| Frame boundary | Mesen calls exec callbacks once per instruction, at the boundary before its opcode fetch (`Debugger::ProcessInstruction`), so the script captures at the first one at or after scanline 241, dot 1 of Mesen's own PPU, after its end-of-frame event. That is the rule `gp-nes` uses with its own PPU ([formats/recording.md](formats/recording.md#frame-boundary)) |
| Frame 0 | Captured when the script loads: after Mesen's reset sequence, before the first instruction |
| Input | Mesen polls its controllers at the start of scanline 241 (`nes.inputScanline`), just before the boundary, and raises an input-polled event; the script sets the next frame's buttons there with `emu.setInput`. Mesen 2.1.1's `emu.setInput` reads the port from its third argument (`LuaApi::SetInput` reads its parameters one place off; with the port only second, both calls land on port 1 and port 2's empty buttons clear port 1's), so the script passes the port as both |
| Lag | A read callback on $4016 and $4017 marks the frame polled; the Mesen recording's input log carries these flags, so the diff can compare lag frames |
| Events | Positions are Mesen's PPU position when its callback ran. Writes to the PPU registers' mirrors ($2008-$3FFF) are folded down to $2000-$2007, as `gp-nes` logs them. OAM DMA's 256 writes to $2004 reach the write callback too; the script skips them, since `gp-nes` logs only the $4014 write that starts the copy. Mesen raises no event for sprite 0 hits or bank switches, so those aren't logged |

`gp` stores the capture as a recording made by runtime `mesen2 2.1.1` at the pinned commit. Its source and power-on state are the original's, and its input log carries Mesen's polled flags. It keeps a frame trace and an event stream but no checkpoints, since only Mesen could restore one. Its states follow a field table of their own (`GPMS`, version 1, in `crates/gp-oracle/src/mesen.rs`), which uses the canonical state's names for every field Mesen shows with the same meaning. These are the conversions.

| Conversion | Why |
|---|---|
| `ppu.scanline` -1 becomes 261 (311 on PAL) | Mesen numbers the pre-render line -1 |
| `cpu.cycles` is Mesen's 64-bit count | The canonical state's field is 64 bits too. Mesen recordings made before 2026-10-28 (state version 1) held the low 32 bits there and the rest in `cpu.cycles64`; record them again |
| `ppu.ctrl`, `ppu.mask` and `ppu.status` are assembled from Mesen's flags | Mesen keeps them as named booleans. PPUCTRL's master/slave bit isn't shown, so it's 0 |
| `prg_ram` is Mesen's work RAM, then its save RAM | Both live at $6000 |
| `cpu.p` is Mesen's as it is | Mesen keeps bit 5 clear; the diff ignores bits 4 and 5 |

Fields only `gp-nes` has, so not compared: `cpu.interrupts`, `cpu.dma`, `cpu.oam_dma_page`, `ppu.io_latch`, `ppu.io_latch_age`, `ppu.nmi_pending`, `mapper.number`, `mapper.mirroring`, the MMC1, MMC3 and CNROM registers (`mmc1.*`, `mmc3.*`, `cnrom.*`; the Lua capture doesn't read Mesen's mapper state yet), `apu.dmc.irq_pending`, `pad1.index` and `pad2.index`. Mesen's state doesn't show them.

The APU's timer counters and sequencer positions (each channel's `timer.counter`, the pulse `duty_step`, `triangle.step`, `noise.shift`, `dmc.bits_remaining`, `frame_counter.cycle`) are kept under `mesen.`-prefixed names, so they aren't paired with `gp-nes`'s either. They differ at power-on, before the game has run (pulse duty step 0 against 6, noise shift register $2000 against $4000, DMC timer 419 against 426), so the two emulators count them from other origins or in other directions. Comparing them needs each convention worked out first, an open question in game-printer.

## The diff

The diff pairs fields by name and width, so it covers exactly the fields both tables have. A divergence is a frame where one of them differs in a way not already reported, and its classes say why.

| Class | Meaning |
|---|---|
| A field name, such as `ram` or `cpu.a` | That field differs |
| `frame-shift` | B's state at this frame is A's at the next or previous frame: a frame was skipped or repeated |
| `boundary-instruction` | The two frames ended an instruction apart: the cycle counts differ by 1 to 7 and S is the same. It explains `cpu.pc`, `cpu.cycles` and the registers an instruction changes. Memory is compared as always |
| `nmi-boundary` | One side ended after an NMI entry and the other before it. Each condition is checked: S is 3 lower on the side that took it, its stack holds the other side's PC (or the next instruction's) and P, its I flag is set, and it is 5 to 14 cycles ahead (the 7-cycle entry, up to 2 cycles sooner because of the alignment, and at most one instruction). It explains S, PC, the cycles, the I flag, and the three pushed bytes of `ram` and no others |
| `ppu-alignment` | The two frame ends are at the same CPU cycle, and the PPU positions differ by exactly the run's alignment |
| `lag` | The two input logs disagree on which frames polled the controllers |
| `events.<kind>` | That kind's events, over the whole run in order, differ in address or value, or one side has more beyond the last frame |

The alignment is how many dots B's PPU runs ahead of A's at the same CPU cycle, measured at frame 1 (frame 0 is the power-on, before `gp-nes`'s PPU settles on its alignment). The two boundary classes also explain the PPU position when it moved 3 dots per cycle between the two ends plus the alignment (within 2 dots), and what the APU frame counter clocks when it is one tick apart. `crates/gp-oracle/src/diff.rs` (`boundary_class`) has the exact conditions.

After a divergence the diff resyncs and goes on to the next:

- A frame shift moves the alignment by one frame.
- When A is a `gp-nes` recording made by this runtime and its ROM is found, A runs live in `gp-nes` (mode `live`). At a divergence with a class beyond the boundary classes, every differing field that can be written into a running console is copied from B's state before the next frame (`gp_nes::poke` writes every canonical field except palette RAM and the mapper fields), so the next frame starts where B's did. When B is another runtime, the timing fields (`cpu.cycles`, the PPU position and frame parity) and what a boundary class explains stay A's: the two CPUs and PPUs are aligned differently, and copying B's position would give A an alignment its own clocks don't have. A field copied 3 frames in a row that still differs is set aside; the count starts again once the two agree on it.
- Other fields, and every field with `--trace-only` or without the ROM (mode `trace`), are set aside with what their difference looks like: an integer's difference (B's value minus A's), or a memory's differing bytes. A set-aside field is reported again when its difference changes, or when a byte outside the set starts to differ, and compared normally once the two agree. A difference that lasts is reported once; one that drifts is reported each time it moves.

The diff compares each event kind over the whole run, so a frame boundary that falls on the other side of an event isn't a difference; the report counts such events as "in another frame". Where events match in the same frame, it shows B's position minus A's, in dots.

The diff runs in about 2 s for 1,800 frames.

### Testing the diff

The tests in `crates/gp-oracle/src/diff.rs` run in `just check`, in well under a second, with hand-made states and our own test ROM, so they need neither Mesen nor a ROM file.

| Test | What must happen |
|---|---|
| A RAM byte flipped at frame 150 | Found at frame 150 as class `ram`, with the address, from the traces and live |
| The low bit of the test program's frame counter ($00) flipped at frame 150, which changes which frames it polls on | Live, the whole divergent state is copied, so it is one divergence; from the traces it keeps diverging |
| Four flips of $00 50 frames apart, then a byte the program never reads | All five found: the copy limit counts frames in a row |
| A skipped frame, a repeated frame | `frame-shift` at that frame |
| A cycle count drifting a cycle a frame | Reported every frame; a boundary class while within one instruction, `cpu.cycles` after |
| An NMI entry on one side, and lookalikes (wrong pushed PC or P, I clear, cycles out of range) | `nmi-boundary` for the real one only; RAM beyond the pushed bytes stays a `ram` divergence |
| Ends an instruction apart, and lookalikes (same cycles with another PC, too far apart, S off by other than 3, a position or frame counter that didn't move with the cycles) | `boundary-instruction` for the real one only |
| A PPU 4 dots ahead from frame 1, and another offset | `ppu-alignment`, and `ppu.dot` for the other |

`scripts/oracle/mutate-diff.py` makes 20 changes to the classifier and resync, each undoing one of those conditions, and checks the tests catch each; on 2026-10-27 they caught all 20.

## Triage

`oracle/triage.toml` holds records of classes. Each names its verdict and date and gives the evidence. Its `sources` list says which registry IDs it applies to, and its `frames` pair which frames; either left out means any. A divergence is triaged when a record of its class covers its source and frame; a class with a divergence no record covers is untriaged, so a later difference in a triaged field isn't accepted in advance. `gp oracle triage <class> <verdict> --note ... [--source ID]... [--frames N|A..=B]` adds a record, or replaces the one with the same class and scope. The file must exist (a missing one is an error, exit 2); `--file` names one to create. Classes are about the emulators, not any game, so the file is in the repo. gameprinter's keeps game-printer's records for gp-testrom; those for other games' sources went with the games.

| Verdict | Meaning | Golden allowed |
|---|---|---|
| `fork-bug` | `gp-nes` (our `tetanes-core` fork) behaves differently from the hardware | No |
| `mesen-difference` | Mesen behaves differently from the hardware, or shows the same behaviour differently | Yes |
| `accepted-configuration` | The two run with different settings, on purpose or because one can't be set like the other | Yes |

`gp oracle golden <gp-nes id> --against <mesen id>` diffs the two (live when it can), and if every divergence is triaged and none is a fork bug, stores a verdict object (the report and the triage records of its classes) and adds an entry with `golden` set to the recordings log, whose notes name the verdict's hash. The verdict object is referenced only from those notes, which `gp store unreferenced` and `gp store gc` count as a use, so it's kept. Otherwise it prints what's missing and exits 1.

## Donkey Kong, 2026-11-13

A 900-frame run of Donkey Kong: Original Edition (start, a walk right, a jump and a walk left) recorded in gp-nes and run in Mesen on linux/arm64 gives the same Mesen recording from gameprinter's `gp oracle record` and game-printer's, and the same diff report (40 divergences shown in 17 classes: the boundary classes, the registers and position at frames whose ends are an instruction apart, `ram`, `prg_ram`, `nametables` and `palette`). None of them is triaged for Donkey Kong yet.

## On an arcade romset

`gp oracle record ID` on a gp-arcade recording runs its input log on its romset in MAME, in the image `just mame-build` makes ([docs/oracle-mame.md](oracle-mame.md)), with `scripts/oracle/mame-record.sh` and `mame-capture.lua` (built into gp, so no checkout is needed). The capture is derived from the ROM, so it is kept under `$GP_HOME/work/oracle/RECORDING-ID/`, never in the store or a repo, with the input log beside it. The romset is the one in `$GP_ROM_DIR` whose identity the recording names (or `--rom`).

`gp oracle diff ID CAPTURE` plays the recording again on gp-arcade and compares every frame with the capture, as gp-arcade's `compare` example does: the RAM record (work, sprite, video, palette and sound RAM), the input ports, the picture's hash and, when the capture has them, the AY writes. It prints the first frames that differ (`--max`), counts the frames that differ in each way, and exits 1 if any does. CAPTURE is a folder name under `$GP_HOME/work/oracle/` or a path.

A MAME capture isn't a store recording, so the field-by-field classes, the triage and `golden` stay the NES's; `gp oracle golden` refuses an arcade recording in words, and `gp oracle check` takes the NES baselines.

On 2026-11-13, a recording of `scripts/oracle/inputs/tapper-first.input` on rbtapper (1,511 frames) ran in MAME 0.276 in 9 s and matched gp-arcade on every frame, AY writes included, as it does against the capture the MAME oracle's first run made. The five-levels recording of Tapper (10,934 frames) matches its MAME capture (`tapper-m3-five`) on every frame too.
