# Test ROMs

Brought from game-printer on 2026-11-13. These are the NES's suites, run on gp-nes. An arcade board is held to MAME instead, frame by frame (`gp oracle diff` against a capture, docs/oracle.md). A Z80 instruction suite (ZEXDOC, ZEXALL) would need a CP/M harness on gp-arcade's Z80 (the program at 0100, BDOS calls 2 and 9 at 0005) and a pinned download, and isn't here yet.

This is the accuracy baseline. The standard NES test ROMs run headless on `gp-nes`, and each run of `just check-testroms` records their results in the metrics history, with the commit. They show how close the emulator under every recording is to the hardware, and where it differs, before a recording runs into the difference.

## What runs

34 suites, 175 ROMs and 176 results (nestest gives two). The manifest, `test-roms/suites.toml`, lists them with their sources and hashes. Two of them are our own: the collection has no MMC1 test that reports a result, so `mmc1_banking` runs a test program we wrote, and `mmc3_banking` runs another for MMC3's banking ([below](#our-mmc1-test-program)).

| Suite | What it checks | ROMs | How it reports |
|---|---|---|---|
| `nestest` | Every CPU instruction, official and unofficial, against a known-good log | `other/nestest.nes` with `nestest.log` | Compared line by line |
| `instr_test` | Official and unofficial instructions, one ROM per addressing mode | `instr_test-v5/rom_singles/`, 16 ROMs | $6000 |
| `instr_test_v3` | The same, from instr_test-v5's predecessor | `instr_test-v3/rom_singles/`, 15 ROMs | $6000 |
| `nes_instr_test` | The same, from the 2005 version | `nes_instr_test/rom_singles/`, 11 ROMs | $6000 |
| `instr_misc` | Absolute,X and branch wrap-around, and the dummy reads of indexed instructions | `instr_misc/rom_singles/`, 4 ROMs | $6000 |
| `instr_timing` | Cycle count of every instruction, and branch timing | `instr_timing/rom_singles/`, 2 ROMs | $6000 |
| `cpu_timing` | Cycle count of every official instruction | `cpu_timing_test6/cpu_timing_test.nes` | On screen |
| `branch_timing` | Branch timing, taken and not, across page boundaries | `branch_timing_tests/`, 3 ROMs | $F8 |
| `cpu_interrupts` | IRQ, NMI and BRK interaction and latency | `cpu_interrupts_v2/rom_singles/`, 5 ROMs | $6000 |
| `cpu_reset` | Registers at power-on and after reset, and RAM kept across reset | `cpu_reset/`, 2 ROMs | $6000, with reset |
| `cpu_dummy_reads` | Dummy reads of indexed and read-modify-write instructions | `cpu_dummy_reads/cpu_dummy_reads.nes` (CNROM) | On screen |
| `cpu_dummy_writes` | Dummy writes of read-modify-write instructions to OAM and PPU memory | `cpu_dummy_writes/`, 2 ROMs | $6000 |
| `cpu_exec_space` | Running code from the PPU and APU register space, with open bus | `cpu_exec_space/`, 2 ROMs | $6000 |
| `ppu_vbl_nmi` | Vblank flag and NMI timing, to the PPU clock | `ppu_vbl_nmi/rom_singles/`, 10 ROMs | $6000 |
| `vbl_nmi_timing` | The 2005 vblank and NMI timing tests | `vbl_nmi_timing/`, 7 ROMs | $F8 |
| `ppu_2005` | Palette RAM, sprite RAM through $2003, $2004 and $4014, VRAM and the read buffer, vblank clear time | `blargg_ppu_tests_2005.09.15b/`, 4 of 5 ROMs | Code on screen |
| `ppu_open_bus` | The PPU registers' open bus, and its decay | `ppu_open_bus/ppu_open_bus.nes` | $6000 |
| `oam_read` | Reading OAM through $2004 | `oam_read/oam_read.nes` | $6000 |
| `oam_stress` | OAM reads and writes through $2003 and $2004 at random addresses | `oam_stress/oam_stress.nes` | $6000 |
| `sprite_hit` | Sprite 0 hit placement and timing | `sprite_hit_tests_2005.10.05/`, 11 ROMs | $F8 |
| `sprite_overflow` | The sprite overflow flag, including the hardware's faulty sprite evaluation | `sprite_overflow_tests/`, 5 ROMs | $F8 |
| `apu_test` | Length counters, frame IRQ, jitter and DMC | `apu_test/rom_singles/`, 8 ROMs | $6000 |
| `apu_2005` | The 2005 length counter, frame IRQ and timing tests | `blargg_apu_2005.07.30/`, 11 ROMs | Code on screen |
| `pal_apu` | The same on a PAL console, with PAL's frame counter rates | `pal_apu_tests/`, 10 ROMs | $F8, powered on as PAL |
| `apu_reset` | APU state at power-on and after reset | `apu_reset/`, 6 ROMs | $6000, with reset |
| `sprdma_and_dmc_dma` | OAM DMA and DMC DMA at once | `sprdma_and_dmc_dma/`, 2 ROMs | $6000 |
| `dmc_dma_during_read` | DMC DMA during reads of $2007 and $4016 and writes of $2007 | `dmc_dma_during_read4/`, 3 of 5 ROMs | On screen |
| `mmc1` | MMC1 (mapper 1) PRG banking, through the multi-test builds of other suites | 9 ROMs from `instr_test-v5`, `instr_test-v3`, `instr_timing`, `cpu_interrupts_v2`, `apu_test`, `instr_misc` and `ppu_vbl_nmi` | $6000 |
| `mmc1_banking` | MMC1's serial port, PRG and CHR modes, mirroring, reset, ignored writes and PRG RAM disable | `gp/mmc1.nes`, built from `test-roms/mmc1/mmc1.s` | $6000 |
| `mmc3_banking` | MMC3's PRG and CHR modes, mirroring, two scanline IRQs a frame and the $E000 acknowledge | `gp/mmc3.nes`, built from `test-roms/mmc3/mmc3.s` | $6000 |
| `mmc3` | MMC3 (mapper 4) IRQ counter, A12 clocking and scanline timing | `mmc3_test_2/rom_singles/`, 6 ROMs | $6000 |
| `mmc3_v1` | The first release of the same tests, whose sixth ROM tests MMC6 | `mmc3_test/`, 6 ROMs | $6000 |
| `mmc3_irq` | The 2005 MMC3 IRQ counter tests | `mmc3_irq_tests/`, 6 ROMs | $F8 |
| `read_joy` | Controller reads through $4016 while DMC DMA steals cycles | `read_joy3/thorough_test.nes` (CNROM) | On screen |

NROM (mapper 0) is covered throughout, since most of the manifest runs on NROM boards. `cpu_dummy_reads` and `read_joy3` are CNROM (mapper 3). The `mmc1` suite's 9 images run their tests from banked PRG ROM, which covers PRG banking only; `mmc1_banking` covers CHR banking, mirroring and MMC1's write quirks. `mmc3_test_2`'s `6-MMC3_alt` and `mmc3_irq_tests`' `5.MMC3_rev_A` test the MMC3A's IRQ behaviour and run with `mmc3_revision = "a"`. `mmc3_test`'s `6-MMC6` runs the same way, since the MMC6 reloads its counter as the MMC3A does. The rest run as MMC3B/C, the default.

Every ROM powers on in the same state as a recording (`PowerOn::default()`: NTSC, RAM zeroed, headless), except for those MMC3 revisions and `pal_apu`, which powers on as PAL.

## How results are read

`gp_nes::accuracy` reads results five ways, named by `check` in the manifest.

nestest runs in automation mode. After power-on the runner sets the program counter to $C000 and runs one instruction per log line. Before each one it writes the line `gp trace instructions` would ([runtime-protocol.md](runtime-protocol.md#instruction-traces)) and compares it with the log's. The text up to the registers (the address, the bytes and the disassembly) must be the same. Then the registers must match, with the PPU's scanline and dot and the CPU cycle count. The first line lines up the cycle count and PPU position, since the log starts after the reset sequence and its PPU is aligned to the CPU one dot differently. Bit 5 of P doesn't exist in a real 6502, and the log always shows it set, as the tracer does. The value the disassembly shows for a PPU or APU register isn't compared, since it's what each emulator's debugger reads there without side effects (nestest.log has FF for the write-only APU registers, and `tetanes-core` 00).

A test in `gp-cli` (`the_instruction_tracer_matches_nestest_log`, part of `just check`) makes the same comparison from the files already in `$GP_HOME/work/test-roms/`, without downloading. When nestest or its log isn't there, or has another hash, it prints a notice and skips, so `just check` still needs no network; run `gp testroms fetch` once to make it count. The first mismatch is reported with both lines. The official-opcode result covers the 5,003 lines before the first unofficial opcode, of 8,991. Both results also need nestest's own result bytes at $02 and $03 to be zero, which adds little, since RAM starts zeroed; the log match is what carries the result. The log was made with RAM zeroed (its disassembly shows memory as that left it, such as `STX $00 = 00` on line 3), so the suite is marked `needs_zeroed_ram` and a run with RAM all ones skips it.

blargg's newer tests write a status byte at $6000 once $6001-$6003 hold the signature DE B0 61: $80 while running, $81 to ask for the reset button, and a result code below $80 when done (0 is a pass). The text they print is at $6004. The runner presses reset 10 frames after a $81 (the tests ask for at least 100 ms) and reports the code and the text.

blargg's 2005 tests (`check = "blargg-2005"`) write the number of each check to $F8 before running it, and 1 once all of them pass. A failing test leaves the number of the check that failed. They never say when they've finished failing, so a failure runs its whole frame budget.

blargg's 2005 PPU and APU tests (`blargg-2005-screen`) keep their result somewhere else in RAM and print nothing but the code, as `$01`, once they finish. The runner reads the screen every 10 frames and stops at the first line that is `$` and two hex digits: 1 is a pass, and 2 or more is the number of the check that failed, which each readme.txt explains.

`read_joy3` and `cpu_dummy_reads` use blargg's older shell on a CNROM board, which has no RAM at $6000, so their result exists only on screen (`screen`). The runner reads the first nametable as text (the console's tiles are ASCII codes) and looks for "Passed", "Failed" or "Error". It also takes the 2005 shell's banners, "PASSED" and "FAILED #2", which is how `cpu_timing_test6` reports. The `dmc_dma_during_read4` tests print "Passed" the same way.

`cpu_reset` and `apu_reset` ask for the reset button ($81 at $6000) between their checks, so they exercise the runner's reset path; both pass.

Timeouts are in frames, never wall time: each suite's `frames` is at least twice what its slowest ROM took on 2026-10-27, and up to 20 times for the quick suites. Running a test ROM needs no canonical state, so it works for any mapper `tetanes-core` supports, while `gp record` still refuses everything but NROM. Two read-only additions to `gp_nes::Nes` make this possible: `peek` reads the CPU address space without side effects (reaching PRG RAM at $6000, which `wram` doesn't), and `reset` presses the reset button. Neither changes how a frame steps or hashes, so `BEHAVIOUR_VERSION` is unchanged.

## Where the ROMs come from

They're never committed. Their licenses vary or aren't stated, and the pre-commit hook refuses NES images anyway. They come from [christopherpow/nes-test-roms](https://github.com/christopherpow/nes-test-roms), the usual collection of the original authors' releases (blargg's, Kevin Horton's nestest and others), pinned to commit `95d8f621ae55cee0d09b91519a8989ae0e64753b` (2022-03-02, the merge that added `instr_test-v5`). The manifest lists the SHA-256 of each of the 174 files used, 173 ROMs and nestest.log.

`gp testroms fetch` downloads each file from `raw.githubusercontent.com/christopherpow/nes-test-roms/<commit>/<path>` into `$GP_HOME/work/test-roms/<path>` and checks its hash. A file already there with the pinned hash isn't downloaded again; one with another hash is replaced. A download with the wrong hash is deleted and reported, and nothing runs from it. That means the pinned source changed upstream or the download was corrupted: review it before changing a pin.

The manifest lives in its own `test-roms/` directory rather than with the games. Every game's `roms.toml` is part of the registry of games you own, which `gp roms scan` matches by headerless hash. Test ROMs are downloads with a URL, a commit and a result protocol, closer to the disassembly pin in `scripts/disasm/verify-smbdis.sh`. `gp` compiles the manifest in, so a change to it needs a rebuild (`just build` does that).

## Our MMC1 test program

Mega Man 2 is MMC1, and the collection's only MMC1-specific ROM, `MMC1_A12/mmc1_a12.nes`, is a display to watch, with no result to read. So `test-roms/mmc1/mmc1.s` is a test program of our own, in ca65, on a board laid out like Nintendo's SKROM: 256KB PRG ROM in 16 banks, 128KB CHR ROM in 32 banks of 4KB, and 8KB PRG RAM. The first byte of each PRG bank is its bank number, and every byte of each CHR bank is its number, so a read says which bank is mapped. It reports with the $6000 protocol, and its result code is the number of the check that failed:

| Code | Check |
|---|---|
| 2 | After the reset stub writes $80, $C000 holds the last bank. This repeats part of check 6; MMC1's power-on state itself isn't tested, since every bank's reset stub writes $80 first |
| 3 | PRG mode 3: $8000 switches through all 16 banks, and $C000 stays the last |
| 4 | PRG mode 2: $8000 stays bank 0, and $C000 switches |
| 5 | 32KB mode: the low bank bit is ignored, and $C000 is the next bank |
| 6 | A write with bit 7 set empties a half-filled shift register and restores mode 3 |
| 7 | CHR 8KB mode: the low bank bit is ignored, and $1000 is the next bank |
| 8 | CHR 4KB mode: $0000 and $1000 switch independently |
| 9 | Mirroring: one-screen lower and upper are different pages, then vertical and horizontal |
| 10 | The second write of a read-modify-write, on the next cycle, is ignored (INC on a ROM byte of $FF) |
| 11 | With PRG RAM disabled (bit 4 of $E000), reads don't see it and writes don't reach it |

The routines that switch the bank at $C000 run from RAM, and every bank ends with a reset stub and vectors, so the program starts the same way whichever bank the console powers on with. `tetanes-core` passes all ten checks. Changing an expected value in a copy of the source makes the program report that check's code, which is how its failure path was tested.

After the checks it runs on, writing the PRG bank, CHR bank 1 and the control register through the serial port every frame, and stores the result and its frame and button counts in internal RAM ($00 to $02). That's what the open baseline `baselines/mmc1` records. Frames end between serial writes, so the shift register and write delay are at rest at every frame boundary; the unit tests in `crates/gp-nes/src/canonical.rs` cover their other values.

It's never downloaded and never committed as a binary. `just build-testroms` assembles every program the manifest lists with a `built_from` source (`gp testroms built` prints them): each `x.s` with the `x.cfg` beside it, into `$GP_HOME/work/test-roms/gp/x.nes`, the path the manifest must give it. `just check-testroms` builds them before running the suites, and `just check` builds them for the open baselines. The manifest pins the build's SHA-256, with `built_from` naming the source. A missing build stops the run with exit 2 and says to run `just build-testroms`. A build with another hash is never used: the source or the assembler changed. After editing the source on purpose, update the pin.

## A second program, for MMC3

The collection's MMC3 tests cover the IRQ counter in depth but not banking or mirroring, so `test-roms/mmc3/mmc3.s` checks those, and runs the scanline IRQ the way a game does. Its board follows TKROM without the battery. PRG ROM comes in 8KB banks and CHR ROM in 1KB banks, 128KB of each, with 8KB of PRG RAM; each bank holds its own number. The result goes to $6000 as in `mmc1_banking`, and to $00 in internal RAM.

| Code | Check |
|---|---|
| 2 | At power-on, $E000 holds the last bank |
| 3 | PRG mode 0: R6 switches $8000 and R7 $A000 through every bank, and $C000 holds the second-last |
| 4 | PRG mode 1: R6 switches $C000, and $8000 holds the second-last |
| 5 | CHR mode 0: R0 and R1 switch 2KB at $0000 and $0800 (the low bit ignored), R2 to R5 1KB each at $1000 to $1C00 |
| 6 | CHR mode 1: the two halves swap |
| 7 | Mirroring ($A000): vertical, then horizontal |
| 8 | With sprites at $1000, a latch of 99 and the counter reloaded in vblank, two IRQs a frame |
| 9 | Writing $E000 disables the IRQ: none in a whole frame. The acknowledge is covered by check 8, whose handler writes $E000 and would otherwise take the IRQ again at once |

MMC3 never switches out $E000, so the whole program lives in the last bank and needs no routines in RAM. `tetanes-core` passes every check, and an edited copy with one wrong expected value fails with that check's number. Once it has passed, the program keeps the IRQ firing. Each frame it sets R6 and R2 to the frame count and takes the mirroring from the A button. `baselines/mmc3` records that part.

## How to run

```
just check-testroms              # build our own programs, then run every suite
just check-testroms mmc3 nestest # some suites
just check-testroms --ram-ones   # every suite, with RAM powered on as all ones
just build-testroms              # build our own programs only
gp testroms fetch                # download only
gp testroms fetch --redownload   # download everything again
gp testroms run --offline        # run what's downloaded, download nothing
```

`gp testroms run` downloads whatever is missing first. A ROM it can't get (offline, say) is skipped with a notice, and the run still succeeds as long as something ran: when every ROM is skipped it exits 2, since nothing was checked. An unbuilt program of our own is different, since building it takes one command, so it makes the run exit 2 with a reminder to run `just build-testroms`. A cached file whose hash no longer matches its pin is never used: online it's downloaded again, and with `--offline` it's reported as a wrong hash. It exits 1 when a ROM fails that isn't listed as a known failure, when a known failure passes, or when a download's hash isn't its pin.

`just check-testroms` isn't part of `just check`, since it needs the network once. It needs cc65 as well, for our own test program (`brew install cc65` on a Mac, `apt install cc65` on Debian or Ubuntu); without it, `just build-testroms` stops with exit 2 and says so. After the first download, `gp testroms run` takes about 30 seconds on an M-series Mac (the debug build that `just` uses).

## Pass rates

Each run records `testroms.<suite>.passed` and `testroms.<suite>.total` for every suite that ran, and `testroms.passed`, `testroms.total`, `testroms.unexpected` and `testroms.skipped` overall, in the metrics history (`gp metrics report` shows them). A `--ram-ones` run records the same names under `testroms.ram_ones.` instead, so the two histories stay apart. A suite that was skipped records no pass rate, so an offline run leaves a gap rather than a zero.

There are no budgets on these metrics. `budgets.toml` is checked against the run group of `just check`, and a budgeted metric that `just check` doesn't record counts as missing and fails it. The ratchet is in the manifest instead: a ROM that fails today is listed with `known_failure`, naming the open question that tracks it, and `known_result`, how its result starts when it fails that way (such as `code 2:`). Only that failure is excused. Any other failure fails the run, including the same ROM failing a different check or timing out, and so does a known failure that starts passing, until its entry is removed.

## Current results

Run on 2026-10-27 at commit `fb23aa1`, on macOS ARM64, with the ROMs downloaded fresh into an empty `$GP_HOME`: 173 of 175 pass. The two failures are the same test in two releases, both known failures. `mmc3_banking` was added after that run and passes (1 of 1, offline, on the branch that added it); the table includes it.

| Suite | Passed | Notes |
|---|---|---|
| `nestest` | 2 of 2 | All 8,991 log lines match, official and unofficial opcodes |
| `instr_test` | 16 of 16 | |
| `instr_test_v3` | 15 of 15 | |
| `nes_instr_test` | 11 of 11 | |
| `instr_misc` | 4 of 4 | |
| `instr_timing` | 2 of 2 | |
| `cpu_timing` | 1 of 1 | |
| `branch_timing` | 3 of 3 | |
| `cpu_interrupts` | 5 of 5 | |
| `cpu_reset` | 2 of 2 | |
| `cpu_dummy_reads` | 1 of 1 | |
| `cpu_dummy_writes` | 2 of 2 | |
| `cpu_exec_space` | 2 of 2 | |
| `ppu_vbl_nmi` | 10 of 10 | |
| `vbl_nmi_timing` | 7 of 7 | |
| `ppu_2005` | 4 of 4 | |
| `ppu_open_bus` | 1 of 1 | |
| `oam_read` | 1 of 1 | |
| `oam_stress` | 1 of 1 | |
| `sprite_hit` | 11 of 11 | |
| `sprite_overflow` | 5 of 5 | |
| `apu_test` | 8 of 8 | |
| `apu_2005` | 11 of 11 | |
| `pal_apu` | 10 of 10 | |
| `apu_reset` | 6 of 6 | |
| `sprdma_and_dmc_dma` | 2 of 2 | |
| `dmc_dma_during_read` | 3 of 3 | |
| `mmc1` | 9 of 9 | |
| `mmc1_banking` | 1 of 1 | All ten checks |
| `mmc3_banking` | 1 of 1 | All eight checks |
| `mmc3` | 5 of 6 | `4-scanline_timing` fails check #2, "Scanline 0 IRQ should occur later when $2000=$08". A known failure |
| `mmc3_v1` | 5 of 6 | The same failure in the first release of the test. A known failure |
| `mmc3_irq` | 6 of 6 | Its `4.Scanline_timing` is the 2005 version, which doesn't check scanline 0 with $2000 = $08 |
| `read_joy` | 1 of 1 | |

The one failing check is in MMC3 IRQ timing. With sprites fetched from $1000 and the background from $0000 ($2000 = $08), the scanline counter clocks on the rising edge of PPU address line A12 during sprite fetches, and on scanline 0 `tetanes-core` raises the IRQ earlier than the hardware does. Super Mario Bros. doesn't use MMC3, so no recording depends on it yet. Kirby's Adventure does (MMC3, splitting the screen with the scanline IRQ), so it's a row in [open questions](open-questions.md) that bites at step 11. It isn't fixed here, since a fix to the PPU's fetch timing changes emulation and needs the Mesen comparison of step 5 to confirm it.

## With RAM powered on as all ones

`just check-testroms --ram-ones` runs every suite again with `ram_init = ones` (CPU RAM, and PRG RAM where the board has it, start as $FF instead of $00). Run on 2026-10-27 at commit `fb23aa1`: 171 of 173 pass, the same two known failures, and nestest is skipped (its log was made with RAM zeroed). No result depends on power-on RAM, though one run time does. `instr_timing/instr_timing.nes`, the MMC1 multi-test build in the `mmc1` suite, passed in 2,313 frames instead of 1,301, still within its budget of 5,000. Every other ROM took the same number of frames. Most of these tests clear RAM before they start (blargg's shell does), so they say little about code that reads RAM it never wrote; the Mesen comparison of power-on state in step 5 is where that shows.

## What isn't run

The pinned collection has 290 ROM images in 62 directories. Every directory whose ROMs report a pass or fail that the runner can read is in the manifest. The rest:

| Directory or ROM | Why it isn't run |
|---|---|
| `apu_mixer/` (4) | Each plays a tone and its inverse, to be heard as near silence; the $6000 code is 0 whatever the sound was |
| `blargg_nes_cpu_test5/` (2) | Multi-test builds of `nes_instr_test`'s 11 tests, which run singly in the manifest. They end with "All tests complete" and no pass or fail marker |
| `blargg_ppu_tests_2005.09.15b/power_up_palette.nes` | Compares the palette at power-on with the values of the author's own console, which the readme says are probably unique to it |
| `ppu_read_buffer/` | Writes its result at $6000, but it's a CNROM board, which has no RAM there, so `tetanes-core` (rightly) keeps none. On screen it prints with its own compressed character set, which the screen reader can't decode. It runs to the end of an 8,000-frame budget without a readable result. Running it needs PRG RAM on a board that has none, a fork change; see open questions |
| `dmc_dma_during_read4/dma_2007_read.nes`, `double_2007_read.nes` | Print the values read and a checksum, with no pass or fail. Their sources list several right answers, which depend on how the CPU and PPU line up at reset |
| `read_joy3/count_errors.nes`, `count_errors_fast.nes` | Count DMC conflicts over 1,000 reads and print the count, with no verdict |
| `read_joy3/test_buttons.nes`, `PaddleTest3/`, `vaus-test/` | Interactive: they ask for button presses or a paddle |
| `MMC1_A12/` | A display to look at, with no result to read; `mmc1_banking` checks MMC1's CHR banking instead |
| `dmc_tests/` (4) | No result anywhere the runner looks: nothing at $6000 or $F8, and a blank screen after 3,000 frames. No readme or source in the collection |
| `stress/NEStress.NES` | A menu of PPU, APU and CPU tests chosen with the controller |
| `240pee/`, `blargg_litewall/`, `dpcmletterbox/`, `exram/`, `full_palette/`, `m22chrbankingtest/`, `mmc5test/`, `mmc5test_v2/`, `nes15-1.0.0/`, `nmi_sync/`, `nrom368/`, `ny2011/`, `scanline/`, `scanline-a1/`, `scrolltest/`, `soundtest/`, `spritecans-2011/`, `stars_se/`, `stomper/`, `tutor/`, `tvpassfail/`, `volume_tests/`, `window5/` | Demos and visual or audio tests: a person judges the picture or sound |
| `other/` (63) | Demos and visual tests, apart from `nestest.nes`. Two (`Streemerz_bundle.nes`, `test28.nes`) are mapper 28, which `tetanes-core` doesn't support |

## Controller 2

No ROM in the collection checks reads of controller 2 through $4017. `read_joy3` and `dmc_dma_during_read4`'s `dma_4016_read` read only $4016. Our own test program covers port 2 instead (`gp_nes::testrom`, described in [recording.md](recording.md)). Its tests check that each port 2 button lands in its own bit, that the two ports don't mix, and that a frame reading only $4017 is a polled frame.

## A test program for sound capture

Sound capture's open tier has a program that isn't a suite, `test-roms/apu/notes.s`. Its NMI plays a made-up eight-note phrase on pulse 1 with a falling volume, a bass note on the triangle every four notes and a noise beat on every note, 12 frames a note, looping. `crates/gp-cli/tests/sound.rs` (in `just check`) assembles and records it for 400 frames, then captures the song and holds it to its log. The same test round-trips the song through FamiStudio text and MIDI ([sound](sound.md)).

