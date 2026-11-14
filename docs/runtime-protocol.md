# Querying, tracing and the runtime protocol

The runtime protocol is the one interface between tools and every way of running a game ([ARCHITECTURE.md](../ARCHITECTURE.md)). The NES runtime (`gp-nes`) and the arcade runtime (`gp-arcade`) implement the operations that don't need a game model, and three commands use them on stored recordings. The model interpreter (`gp-interp`) implements the protocol over a game model, reading its state by model path ([formats/model-path.md](formats/model-path.md)).

Brought from game-printer's docs/runtime-protocol.md. The protocol (`game-printer:runtime@0.4.0`), the selectors, the conditions and the trace layout are unchanged. What changed: the examples use Donkey Kong instead of the Super Mario Bros. baseline, an arcade section is added, and the cost tables are game-printer's measurements, whose example program (`protocol-costs`) isn't here.

| Command | Does |
|---|---|
| `gp query` | Reads console state at any frame |
| `gp run-until` | Finds the first frame where a condition over that state holds |
| `gp trace instructions` | Prints every CPU instruction in a window of frames |

Everything below reads the NES runtime's raw console state: RAM, CPU registers, the PPU. The interpreter takes model paths such as `entities.player[0].x` instead, in the grammar the selectors are a subset of. An arcade recording has selectors of its own ([Arcade](#arcade)).

## From the command line

Each command finds the recording by ID prefix. `gp query` and `gp run-until` read the frames they need from the recording's trace when it has them, checking each state against its frame hash, and need no ROM or console for it. They replay instead when a selector reads `bus[...]` (which goes through the cartridge's mapper), when `run-until` may run past the recording's last frame, when the trace follows another field table version, when another runtime made the recording (so a runtime that now reaches other states still says so), or when given `--replay`; the JSON's `read_from` says which (`trace` or `replay`). Replaying, and `gp trace instructions` always, finds the ROM in `$GP_ROM_DIR` by hash (or takes `--rom`, which is checked against the recording either way), restores the nearest checkpoint at or before the frame it needs, and replays from there. Frames are numbered as in [the recording format](formats/recording.md#frame-boundary): frame N is the state after N frames, and also the frame that produces it, which runs with input log entry N-1. Frame 0 is power-on. So `gp query --frame N` reads state N, while `gp trace instructions --frames N` traces the instructions that produce it. `gp events --frames N` lists the events those instructions make.

Quote every selector and condition on the command line, as the examples here do: zsh treats an unquoted `ram[0x0053]` as a glob pattern and stops with "no matches found" before `gp` runs.

```sh
gp query 76c9 --frame 1200 'ram[0x0053]' 'ram[0x0055]' cpu.pc
gp run-until 76c9 'ram[0x0053] == 3' --show 'ram[0x0055]'
```

These read a Donkey Kong (JU) recording, such as the 5658S run of [corpus.md](corpus.md). In Donkey Kong `$0053` is the stage (1 for 25m, 3 for 75m, 4 for 100m) and `$0055` the lives. `gp query` prints one line per value, its path, the value and the value in hex (`ram[0x0053] = 1 (0x01)`). `gp run-until` prints the frame where the condition first holds, then each `--show` value at that frame. `run-until` checks the condition at the starting frame (`--from`, default 0) and after each frame, up to `--max-frames` or the end of the recording. Past the end of the input log, the last input stays held. A frame ends just as the NMI is taken, so `cpu.pc` is often the NMI handler's first address.

```sh
gp trace instructions 76c9 --frames 100..102 --out trace.log
```

`--frames 100..102` traces frames 100 and 101, and `100..=102` adds 102. `100..` runs to the last frame, and `100` is one frame. `gp events --frames` takes the same forms. An empty or reversed range is an error, as is one that goes past the last frame. Frame 0, which no instructions produce, is left out. Without `--out` the trace goes to standard output.

The exit codes are the ones in [CLI.md](../CLI.md). A command exits 1 when `run-until`'s condition never held, or when the state it reached isn't the one recorded, which happens when a recording from another runtime no longer replays the same. It exits 2 for errors such as a bad selector or a frame past the end, and 0 otherwise.

## Selectors

A selector names one value or a range of values. The names are the canonical state's fields ([formats/recording.md](formats/recording.md#canonical-state)), plus three of our own:

| Selector | Reads |
|---|---|
| `ram[0x0770]`, `ram[0x0700..0x0710]`, `ram[*]` | Internal RAM: one byte, a half-open range, or all 2,048 |
| `oam[...]`, `palette[...]`, `nametables[...]`, `chr_ram[...]`, `prg_ram[...]` | The other memories in the canonical state, indexed the same way |
| `cpu.pc`, `cpu.a`, `ppu.scanline`, `ppu.dot`, `apu.noise.shift`, ... | Any integer field of the canonical state |
| `cpu`, `ppu`, `apu.pulse1`, ... | Every integer field under that name |
| `bus[0xfffc]`, `bus[0x8000..0x8010]` | The CPU's address space as the CPU sees it, read without side effects (a read of $2002 here doesn't clear vblank) |
| `nmi.cpu.cycles`, `nmi.ppu.odd_frame`, `nmi.ram[0x18]` | Any canonical state field as the CPU takes the next NMI after the frame (gp-nes runs a copy of the console on to it), whatever `read_at_nmi` says |
| `nmi.return_pc`, `nmi.status`, `nmi.vblank_dots` | That NMI's pushed return address and status (where the code it stopped goes on, and its carry) and its entry in PPU dots after the vblank (scanline 241, dot 1), for a game whose idle loop's place no RAM byte holds (a state mapping's `field.machine`, [cycles.md](cycles.md)) |
| `frame` | The frame number |

Numbers are decimal, or hex after `0x` or `$` (quote `$` from the shell). Each result is an entry, a path and a value. Indexes in paths are written as `0x` and four hex digits, so `ram[$53]` comes back as `ram[0x0053]`.

The grammar is a subset of the model path grammar ([formats/model-path.md](formats/model-path.md)): dotted names, each optionally followed by an index. Ranges (`[a..b]`) are new; `[*]` is the model grammar's wildcard; `[name=value]` filters are left to the model. `crates/gp-runtime/src/select.rs` has the parser.

```text
selector  := segment ("." segment)*
segment   := name ("[" index "]")?
index     := number | number ".." number | "*"
number    := "-"? (digits | "0x" hex | "$" hex)
```

## Conditions

A condition compares selectors with numbers or with each other, combined with `&&`, `||`, `!` and parentheses. A bare operand is true when it isn't zero, and `& mask` keeps only some bits:

```text
ram[0x0053] == 1
ram[0x0053] == 3 && ram[0x0055] >= 2
ppu.status & 0x80
!(cpu.pc < 0x8000)
```

```text
condition := and ("||" and)*
and       := unary ("&&" unary)*
unary     := "!" unary | "(" condition ")" | compare
compare   := operand (("==" | "!=" | "<" | "<=" | ">" | ">=") operand)?
operand   := (selector | number) ("&" number)?
```

Every selector in a condition must name exactly one value. Conditions are checked at frame boundaries (the start of vertical blank), not between instructions.

## Instruction traces

Instruction traces aren't stored; they're made on demand from checkpoints ([formats/recording.md](formats/recording.md#instruction-traces)). Each line is the CPU before the instruction runs, laid out like nestest.log so it reads side by side with other emulators' traces. This is our test program (`gp_nes::testrom`):

```text
; frame 70, pads 80 00
C017  E6 00     INC $00 = 42                    A:43 X:00 Y:00 P:A4 SP:FC PPU:241, 30 CYC:2052489
C019  A5 00     LDA $00 = 43                    A:43 X:00 Y:00 P:24 SP:FC PPU:241, 44 CYC:2052494
...
C014  4C 14 C0  JMP $C014                       A:44 X:00 Y:00 P:A4 SP:FF PPU:241,  0 CYC:2082260
; NMI
```

| Column | Meaning |
|---|---|
| `C017` | Program counter |
| `E6 00` | The instruction's bytes |
| `INC $00 = 42` | The instruction, with what its operand comes to: `@` the address an indexed operand reaches, `=` the value there before it runs. As in nestest.log, `LDA ($80,X) @ 80 = 0200 = 5A` shows the pointer's zero page address, the address it holds and the value, and `LDA ($89),Y = 0300 @ 0300 = 89` the address the pointer holds, that plus Y and the value. Unofficial opcodes are marked `*` |
| `A X Y P SP` | Registers, in hex. Bit 5 of P is always set, as nestest.log shows it: a 6502 doesn't store it, and it reads as 1 when P is pushed |
| `PPU:241, 31` | The PPU's scanline and dot |
| `CYC` | CPU cycles since power-on |

In game-printer, `gp testroms` ran nestest through this same tracer and compared every line with nestest.log, the disassembly as text and the registers as numbers, and all 8,991 lines matched. The one column left out is the value after `=` for a PPU or APU register ($2000-$401F), which is what each emulator's debugger shows for a read without side effects: nestest.log shows FF for the write-only APU registers, where `gp` shows 00. In gameprinter the comparison is `gp_nes::accuracy::run_nestest`, whose line reader the unit tests check; `gp testroms run` runs it on the downloaded nestest ([test-roms.md](test-roms.md)).

Lines starting with `;` are comments: a header for each frame with its input, and `; NMI` or `; IRQ` after an instruction that was followed by an interrupt, so the next line is the handler's first. The frame ends at the first instruction boundary at or after vblank starts, so whether the NMI that follows falls in this frame or the next varies. When the instruction ending the frame before the window took an NMI, the window starts inside the handler with no `; NMI` line.

No fork patch was needed. `gp-nes` already runs the CPU one instruction at a time to find the frame boundary, so the trace reads the console between instructions through `Nes::step_with`. With tracing off that callback does nothing and compiles away: in game-printer replays ran at the same speed before and after (1,400 to 1,490 frames per second either way, in alternating runs), and the stored recordings and both baselines kept their hashes.

An arcade recording's trace prints both Z80s in time order; [recording.md](recording.md#arcade-recordings) shows its lines.

## Arcade

`gp_arcade::protocol::ArcadeRuntime` puts Tapper behind the same protocol, so `gp query`, `gp run-until`, `gp trace` and `gp events` read an arcade recording too. An input frame's `buttons` low byte holds the controls, bits 0 to 6 coin, start, right, left, down, up and tap; higher bytes, axes and the reset button are refused, since the board has none of them.

| Selector | Reads |
|---|---|
| `frame` | Frames run since power-on |
| `ram[A]` | The main CPU's address space at `A` (0 to 0xFFFF) as the CPU reads it, without side effects: work RAM at 0xE000, sprite RAM at 0xE800, video RAM at 0xF000, the program below |
| `bus[A]` | The same read as `ram[A]`, under the name the NES gives its CPU's address space, so a selector written for one platform reads on the other |
| `sound_ram[A]` | The SSIO board's 1 KB of RAM, by offset (its Z80 sees it at 0x8000) |
| `palette[A]` | The 128 bytes of palette RAM, which the main CPU only writes (at 0xF800) |
| `ports[N]` | The five input ports as the game read them in the last frame |
| `cpu.pc`, `sound_cpu.pc` | The two Z80s' program counters |

The canonical state is the machine's snapshot: both CPUs, the timers, every RAM and the last frame's picture. Each frame stepped gives one `frame` event with a `polled` flag, false on a frame where the game didn't read its inputs. With `set("ay_events", true)` it also gives an `ay-write` event for each AY register the sound CPU wrote in the frame, with `chip`, `reg` and `value`. Its capabilities are `snapshot` and `fork`, as `gp-nes`'s.

## The protocol in Rust

`wit/runtime/runtime.wit` (`game-printer:runtime@0.4.0`) is the definition. `crates/gp-runtime` mirrors its `session` interface as a Rust trait named `Runtime`, with a Rust type for each WIT type. `gp_nes::protocol::NesRuntime`, `gp_arcade::protocol::ArcadeRuntime` and `gp_interp::Interp` implement the trait, and so do a few runtimes inside `gp-cli` that wrap the original or a model for a command. A test in `gp-runtime` (`tests/wit.rs`) parses the WIT and compares it with the trait's source and the types. It fails on any difference in the methods and their parameters, or in the fields and cases of the types.

game-printer's AGENTS.md said to generate bindings from WIT rather than write them; here the trait is written by hand ([ARCHITECTURE.md](../ARCHITECTURE.md)), since nothing crosses a component boundary yet, so there's nothing to bind. Every caller (the CLI, the tests) is Rust in this workspace, and calls `gp-nes` directly. Building `gp-nes` as a component would need `wit-bindgen`'s export glue plus a host, and would copy snapshots, query results and events across the boundary on every call, where `fork` is now a 5 us clone of a Rust value. The component is generated from the WIT once a runtime has to load as one, and it wraps the Rust runtime. The sync test is what keeps that cheap.

Version `0.4.0` (2026-10-29) adds `load-state(state)` and the capability `load-state`, with the `load-note` record and the `load-how` enum. It loads a canonical model state (the bytes `canonical-state` returns, `gp-sim`'s `GPSM`) into any runtime of the same model. It returns a note for each part it approximated or couldn't load (`path`, `how` as `loaded`, `approximated` or `missing`, and `why`); a runtime that loads the whole state returns none. Where `restore` takes only the same runtime's opaque snapshot, `load-state` takes a state another runtime made or a tool built: `gp_interp::translate` builds one from the original's state through the state mapping, with notes of its own. What runtimes must change: a model runtime implements it and sets the flag (the interpreter does: its canonical state is its snapshot, so it loads in full); one with no model refuses it and leaves the flag unset (`gp-nes` refuses, pointing at `restore`). Tools that build input frames are unaffected.

Version `0.3.0`, from 2026-10-27, adds one field: `input-frame.reset`, the reset button pressed at the start of the frame, so a run whose input log resets (input log format 1, such as an imported TAS movie) replays through the protocol exactly. A runtime presses it before the frame runs, and holding the last input past the end of a run doesn't press it again. The tools that build input frames from a log (`gp run-until`) copy it. The interpreter starts the game over from its power-on state on a reset.

Version `0.2.0` changed two functions of the first draft (`0.1.0`), which nothing had implemented. Both `step` and `run-until` now return results, since a run can fail partway when an NES program executes a jam opcode and stops the CPU, and the result of `run-until` is an `until` record that says whether the condition was met.

How the NES runtime implements each operation:

| Operation | `gp-nes` |
|---|---|
| `step(inputs)` | One frame per input. `buttons` holds port 1 in the low byte, then port 2; ports 3 and 4 and analog axes are refused |
| `run-until(condition, inputs, max-frames)` | As in [Conditions](#conditions). Inputs past the end hold the last one |
| `frame()` | Frames since power-on |
| `snapshot()` / `restore(snap)` | `tetanes-core`'s save state, behind a header with the frame, the held buttons and the runtime version. A snapshot from another runtime version is refused |
| `fork()` | A clone of the whole console |
| `canonical-state()` / `state-hash()` | The canonical state that recordings hash. `state-hash` is the first 8 bytes, little-endian, of the BLAKE3 hash recordings store |
| `query(selector)` | As in [Selectors](#selectors) |
| `sample(selectors, inputs, frames)` | Runs `frames` frames and reads every selector after each: one row per frame |
| `entities-at`, `patch`, `explain` | Empty or rejected: these need a model or a state mapping |
| `set(path, value)` | `ram[addr]` only, for test setup |
| `load-state(state)` | Refused: the console has no model state |
| `capabilities()` | `snapshot` and `fork` |

### Events

Each frame stepped gives the events of [the event stream](formats/recording.md#event-stream) (interrupts, sprite 0 hits, bank switches, controller reads and writes, writes to PPU and APU registers) and then one event of kind `frame`, with the frame's number and a `polled` flag that is false on lag frames. `step` and `run-until` return the same events a recording stores for those frames, and a test in `gp-nes` checks it. Kinds have one naming, the stream's (`ppu-write`, not `ppu.write`). The WIT's comment on `record event` lists them, and another `gp-nes` test fails if it misses one.

| Event field | Holds |
|---|---|
| `frame` | The frame the event happened in |
| `kind` | The event stream's kind name, such as `nmi` or `ppu-write`; `frame` for the end of a frame |
| `entity` | Empty until a state mapping names entities |
| `data` | For a stream event, `scanline` and `dot`, then `addr` and `value` (an interrupt has only `addr`, the handler's address, and a sprite 0 hit neither). For `frame`, `polled` |

Within a frame, events come in the order they happened, and the `frame` event comes last.

## What it costs

game-printer measured these on Super Mario Bros. in a release build on an M-series Mac, with its `protocol-costs` example over the 1,800-frame baseline run; neither the example nor the baseline is in gameprinter. Each figure is the console as it is at the end of that run.

| Operation | Time | Size |
|---|---|---|
| `step` | about 700 us a frame (1,380 to 1,490 frames per second) | |
| `snapshot` | 1.3 us | 21,586 bytes |
| `restore` | 42 to 48 us | |
| `snapshot` then `restore` | 43 to 47 us | |
| `fork` (a clone) | 5 to 6 us | |
| `canonical-state` | 3 us | 12,737 bytes |
| `state-hash` | 8 to 10 us | |
| `query("ram[0x0770]")` | 0.3 us | |
| `query("cpu.pc")` | 3 to 4 us (it encodes the canonical state) | |
| Restore and replay 59 frames, the worst case with checkpoints every 60 | 38 to 44 ms | |

Forking by clone is about 8 times cheaper than a snapshot and restore, because the clone copies memory where the restore deserializes it, so `fork` clones.

Whole commands, timed from the shell (wall clock, including starting `gp` and finding the ROM):

| Command | Time |
|---|---|
| `gp query` at frame 1680 (a checkpoint) | 20 ms |
| `gp query` at frame 1739 (59 frames after one) | 60 ms, of which 38 ms is the seek |
| `gp query` at frame 1739, from the trace | 0.2 to 0.45 ms to read the state; the command is under 1 ms before process start |
| `gp trace instructions --frames 100..102` | 64 ms for 19,445 instructions, 1.9 MB of text |

Reading from the trace was measured on 2026-10-27 on a recording of that run, in a release build:

| Command | From the trace | With `--replay` |
|---|---|---|
| `gp query ID --frame 1739 'ram[0x0770]'` | 0.66 to 0.87 ms (the state: 0.22 to 0.45 ms) | 44 ms (the seek: 43 ms, 59 frames) |
| `gp run-until ID 'ram[0x075a] == 0 && frame > 1000'` (holds at frame 1060) | 11 ms | 733 ms |

The trace path decodes one 64-frame chunk and hashes the state (about 10 us) for each frame it reads. Wall clock with process start, by `hyperfine`, was 34 to 47 ms for the query and 51 ms for the `run-until`, against 63 to 75 ms and 915 ms replaying. `gp trace instructions` still restores a checkpoint, because it runs the CPU. The canonical state can't start a console, since it leaves out `tetanes-core`'s master-clock bookkeeping and settings.

With checkpoints every 60 frames, the worst-case seek is about 40 ms and the whole command 60 to 65 ms, well inside a second, so the interval could grow tenfold before jumping to any frame took a second.

## Not done yet

- A long-lived session that keeps a console open between queries. Each command now opens the recording again.
- The component build of the protocol, described above.
