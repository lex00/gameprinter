# The package scan

`gp package scan DIR` sorts everything in a game's folder into what its package (the published version of that folder) may hold and what must be lifted again from the person's own ROM. Each piece of a game's model ships in its package, or ships as a recipe that lifts it from the ROM with a hash that checks the result. Nothing may simply be missing. It came from game-printer's docs/package-scan.md, where it read a template folder; here it reads a package folder in the games checkout and its `[[rom.entry]]` list, and `gp package rederive` is part of the scan only.

```text
gp package scan ../games/donkey-kong [--rom ROM] [--wasm MODULE] [--baseline SCAN.json]
gp package scan ../games/donkey-kong --json > scan.json
```

The ROMs compared with are each ROM entry of `package.toml` found by its hash in `$GP_ROM_DIR`, or the file `--rom` gives. With no ROM found the report says plainly that nothing was compared, and the only items it can find are the ones that need no ROM. The ROM is read locally and nothing of it is printed: a match is reported by where it is in the file, where it is in the ROM, and its length.

A package with an open version (an `[open]` table in `package.toml`, naming a model in a folder of the package with its own `template.toml`) is scanned in two parts: the ROM game at the root, and the open version in its folder. An original (`source.how = "authored"`) isn't compared with its own image, which is built from its files.

## The buckets

| Bucket | What goes in it |
|---|---|
| ours | Facts, rules, state machines, behaviour written as model fields, hook code that describes behaviour, themes with original art, clip and probe recipes, scores, docs |
| reference | Code addresses, routine names, RAM addresses, mapping rows. They point at the ROM and copy nothing |
| from the ROM | A value that is the ROM's content, present as a recipe: an asset map entry (where and how to decode) and a model field that names it (`assets.*.from.asset`, `ext_tables`), checked by the lifted files' hash |
| needs rewrite | ROM content in the package without a recipe (a table typed out as numbers in model.toml, in hook code or in a comment, including short runs, reordered or scaled copies and values split over fields), a line of a lifted file pasted in, a model field naming an asset the recipe doesn't make, or hook code that transcribes a routine |

What is counted depends on the file:

| File | Unit | Bucket unless a detector flags it |
|---|---|---|
| `package.toml`, `roms.toml`, `template.toml` | field | ours |
| `model.toml` (or the open version's model), `delta.toml` | field | from the ROM for `assets.*.from.asset` and `*.ext_tables.*`; reference for a key in the reference list or a string of addresses only; else ours |
| `mapping.toml` at the part's root | field | reference |
| Other TOML (probes, corpus, profiles, fidelity, observe) | field | reference by key or address string, else ours |
| `authored.toml` | field | ours, never compared with the part's own image |
| `.rs` files | function, const or static | ours |
| `clips/*` and `inputs/*` (clip recipes), `evidence/*` and `baselines/*` (scores), `themes/*`, `.md` and `.txt` | file | ours |
| An asset map a ROM entry names | asset | from the ROM |

The reference keys are `routine`, `routines`, `addr`, `address`, `addresses`, `ram`, `pc`, `reads`, `writes`, `rom_offset`, `entry_pc` and `code`. A string of addresses only is one such as `"$C890"`, `"$FA48-$FE58"` or `"0x0203"`.

A unit is needs rewrite when a needs-rewrite item covers any of its bytes. An item that covers no unit (a match in a comment between fields) counts as a unit of its own.

An arcade entry's asset map names pictures by graphics region and index, not by spans of one ROM, so it marks no bytes as the ROM's own: its chips are compared whole.

## Detectors

### ROM content in text

The code is `crates/gp-cli/src/package/scan/rom_match.rs`. Every file's integer tokens are read in order, wherever they are (values, arrays, comments, strings): decimal, `0x` or `$` hex, `0b` binary, with an optional Rust type suffix, and a leading `-` after a space, `=`, `,`, `[` or `(`. Floats, digits inside identifiers (`bounce_right1`) and version numbers aren't tokens. Two streams are read from them: `bytes`, each value from -128 to 255 as one byte, and `words`, where a value over 255 or written in hex with more than two digits is two bytes, low first (a pointer table). Because the stream skips everything between tokens, a table split over fields or lines is still one run.

| Detector | Finds | How |
|---|---|---|
| exact | The ROM's bytes in the same order | Seeded by 3-byte windows of the ROM, grown while the stream follows the ROM; up to 2 other numbers may sit between matched ones when the next 3 match again |
| reordered | The ROM's bytes in another order | Within a group (an array, an inline table, a table's fields between headers, a bracketed list in code), windows of 6 to 16 values whose multiset is a ROM window's |
| scaled | Each value k times the ROM's plus one offset | Within a group, windows of 6 to 16 values whose steps are k times a ROM window's steps, k one of 1, 2, 3, 4, 5, 6, 8, 10, 16, 32, 64, 128 or 256 either sign, the ROM read as unsigned or signed; k = 1 with offset 0 is the exact detector's |

A match counts only when it is unlikely by chance. Its chance is the expected number of chance matches over the whole scan: the ROM's size, times the chance of the values (each value's chance the larger of its frequency in the ROM and its step's frequency from the value before, so the ROM's ramps and counters count as likely), times the places tried (every number in the three detectors, against each ROM, and for reordered and scaled every window length). The thresholds:

| Setting | Value |
|---|---|
| The largest chance that counts (`ALPHA`) | 0.01 |
| The fewest values in an exact run | 4 |
| The fewest values in a reordered or scaled window | 6 |
| The longest window tried | 16 |
| Other numbers allowed between matched ones | 2, each adding a factor of 3 to the chance |
| Too regular to count | fewer than 3 distinct values, one step repeated (a ramp, with or without gaps), or distinct values that are a block of consecutive values in any order (a metatile's tile numbers) |
| Reordered and scaled look in | PRG only: CHR is pictures, whose regular rows line up with anything reordered or scaled, and a picture typed out is found exactly |

Each match's report gives its chance, so a reader can tell a near miss from a certain copy. Lines of 24 characters or more of the lifted files found in a package file are needs rewrite too (`template::shared_lines`, as `gp template served` uses).

### Transcriptions

The code is `crates/gp-cli/src/package/scan/transcribe.rs`. The rule for hook code is "model what the code does, never transcribe how". A hook function cites the routine it models by its name (`fn e166`, `fn dfe4_body`) or by a `$XXXX` address of $8000 or more in its doc comment. For each cited routine (up to 4), two sequences are compared:

| Side | Its operands |
|---|---|
| The function | The numbers in its body in order (literals, and addresses written in its comments), less 0 and 1 and the literals added to an accumulator |
| The routine | Each 6502 instruction's operand in address order (immediates, the addresses read and written, JSR targets), walking the routine in the ROM from its address through branches and JMPs within $400 bytes of its start, to its RTS, RTI or BRK, at most 2048 instructions |

| Signal | When it fires |
|---|---|
| same-order | The longest common subsequence of the two is at least 6, at least 0.4 of the shorter, and at least twice the longest common subsequence with the routine's operands reversed. The reversed sequence has the same values in another order, so the signal is about order, which is what a transcription keeps |
| cycle-count | The function adds a literal of 2 or more to an accumulator (`let mut c = 6; c += 3;`) 4 or more times: it counts the routine's instruction cycles, which is how the routine runs, not what it does. An add of 1 is a counter's or an index's step (`i += 1`): no 6502 instruction takes fewer than 2 cycles (the test `an_index_step_is_not_a_cycle_count`) |

A function with either signal is needs rewrite. The fix is to describe what the routine does (the states it moves between, what it tests and what it changes) as model rules or `ext_params`, and to take a cost from the ROM's code: the hook answers the routine's branches by their addresses from model state and `gp_cycles::walk` adds the cycles from the lifted code ([cycles](cycles.md#costs-from-the-roms-code)), with a draw it doesn't model kept as a measured fact. A constant the code holds as an immediate (a compare's operand, a place it loads) is read from the lifted code at its operand's address, not typed out.

### Modules and other binary files

Every `.wasm` in the folder (a `target` or `.git` folder aside), and each module `--wasm` names, is read by its data segments and custom sections, and every other file that isn't text is compared whole. Any run of 32 of a ROM's bytes in one is needs rewrite (`served::runs_of_rom`, as `gp template served` reads files). A built module of the game's hooks is checked this way before it goes anywhere.

## Rederive

The scan rebuilds everything in the from-the-ROM bucket from the package alone and each ROM found: for each ROM entry, its asset map checked against the entry's `asset_map_sha256`, the lift's digest against its `lifted_sha256`, the lifted files encoded back and compared with the ROM byte for byte, and every asset the model names found in the lift. A missing pin is a problem: a lift with no hash to check it against is trusted, not checked.

A package is complete when it has no needs-rewrite item and the lift from the ROM passes with every hash present. `gp package scan` exits 1 when the package isn't complete, or with `--baseline` only when it finds a needs-rewrite item the baseline doesn't list (or ROM bytes in a module).

Not here yet: `gp package rederive`, the rebuild on its own.

## In the gate

`gp model gate` scans the game's folder at the tip before a patch and again after it, with the module the patch builds. The scan before is counted by file. A file with more needs-rewrite items after the patch than before fails the try (`gp package scan: new needs-rewrite items: ...`), and so does a scan that fails where the tip's passed. The items the tip already has don't fail it, so the gate doesn't block every patch while they are rewritten. A new game has no tip to scan, so its first patch is held to nothing here.

The pre-commit hook scans too: in a repo with package folders (the games checkout), every `package.toml`'s folder is scanned as staged, from a copy of the index, and the commit is refused when a scan fails. With no ROM on the machine the scan says nothing was compared.

## Limits

- The chance model treats the ROM's bytes as independent beyond one step, so a ROM with long regular structure can give a match below `ALPHA` by chance. Each match's chance is in the report.
- A routine is read from the power-on view of PRG ($8000 to $FFFF): a routine in a switched bank of a banked board is read from whatever the power-on view holds there.
- A function that names fields (`s.time`) where the routine names addresses (`$8A`) is compared by its literals and comment addresses only. Its cycle counting still shows.
- Theme pictures, clip presses and scores are counted as whole files, so a match in one makes the whole file needs rewrite.
