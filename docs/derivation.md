# Derivation

A game's model (Moss Hop's template, a game's package) holds facts a person wrote by reading the game's source or disassembly, with evidence that cites the file and line. Derivation reads those facts back automatically. It starts from the cited lines and the code around them, then turns to the ROM's data through the asset map. After that come recordings and other derived values. A fact that a deriver reads back can be checked on every run and filled for the next game without a person; a fact that none reads back says what capture still needs.

Ported from game-printer in wave 5; what changed is at the end. Every deriver below works in a first form on any NES game with its source or disassembly, an asset map and a state mapping. game-printer's measurements on other games are in its own docs/derivation.md.

## The command

```text
gp model derive <model> [--from evidence|code|data|recordings|all] [--assets DIR] [--recording ID...] [--frames A..B] [--root DIR] [--no-questions] [--list] [--json]
```

It derives every fact the derivers in `--from` can (all by default), compares each with the model, and prints the counts and every fact that differs, is new, or is agreed but contradicted (`--list` prints every fact). Each fact gets one status:

| Status | Meaning |
|---|---|
| agree | A derived value equals the model's |
| differ | The one value derived for it isn't the model's; the proposed value is shown |
| new | A deriver proposes a path the model doesn't have (a measured acceleration) |
| underivable | Nothing derived it; the class that could, and why |

An agreed fact that another deriver reads differently (a recording that measures a stated speed otherwise) is listed as "agreed but contradicted", with the other value. Each derived value carries its evidence (the ref it was read at, with the pin), its encoding (`16ths`, `signed 256ths`, `the constant ENTITY_BATTON`) and a confidence. A match counts as an agreement at 50 or more; below that it is reported as possibly chance.

Where a derived value differs, the command asks: a `disagree` question with the ID `derive:<path>` goes into the directory's `questions.toml`, with the model's value and the derived one as candidates, and `gp model questions --answer` writes the chosen one into the model ([mechanics](mechanics.md), "Questions"). That is how derivation feeds the refinement loop. `--no-questions` writes nothing.

Its JSON gives the score: `facts`, `hand_written`, `derived`, `agree`, `differ`, `new`, `contradicted`, `derivable_share` (derived over hand-written) and `agree_share` (agreed and not contradicted, over derived). Hand-written facts are those whose evidence is `stated` or `inferred`; `observed` facts were measured to begin with.

## The classes

Every fact is in one class by how it is derived. The first deriver that agrees names it, in the order (a), (b), (c), (d); an underivable fact gets the class that could derive it, or (e).

| Class | Deriver | What it reads |
|---|---|---|
| (a) evidence | `--from evidence` | The cited line: a constant (`NAME = value`), an immediate operand, a data row, the table a label starts (up to the next global label), the table an operand names, a number in the line's comment (a person's word, at lower confidence, and as sure as the line when the ref quotes that comment), and a constant in a cited file whose name holds the fact's own name |
| (a) code | `--from code` | What the code before the cited line computes there, by constant propagation through its routine, or every value a cited routine stores, loads from a table or compares with |
| (b) data | `--from data` | The lifted rows of the table a cited label names, through the asset map's `label`; the asset slots against the map (source, asset, kind); the generator scripts the evidence names (`scripts/model/*.py`), rerun over the lifted assets and compared fact by fact |
| (c) recordings | `--from recordings` | The feel parameters `gp model propose` measures from the recordings' tracks ([mechanics](mechanics.md), "Measuring feel") |
| (d) derived | with any of them | Values built from others: a box's width and height from a row of left, top, right and bottom edges; a rate the code doubles |
| (e) | | Not derivable today, with the reason |

### Reading a number

A raw number reads as a model value in one of the encodings NES games use: whole and signed (two's complement byte or word), sixteenths and signed sixteenths, and 256ths, signed 256ths (an `8.8` word) and 4096ths (`gp_derive::encode`). Mario's walk speed of `1.5` is read from `MaxRightXSpdData` (`SMBDIS.ASM:6029`) as sixteenths. His gravity of `0.5625` from `FallMForceData` (`SMBDIS.ASM:6017`) as 256ths and his acceleration from `FrictionData` (`SMBDIS.ASM:6033`) as 4096ths. Moss Hop's jump speed of -5 from `JUMP_SPEED` (`games/mosshop/mosshop.s:33`) as signed 256ths. When two encodings of two numbers both give the value, the quantity's name breaks the tie: gravities, accelerations and forces prefer 256ths and 4096ths. Speeds and falls prefer whole numbers and sixteenths.

A small number matches something by chance in most routines and tables, so confidence falls with it: a whole number up to 8 counts only when the cited line itself is that constant or immediate, or the entry cites three numbers or fewer; a whole value read through a fraction (64 sixteenths as 4) counts less than the same number written whole; a fraction read from a small raw number in a large table counts as chance.

### Differences

A fact differs when its evidence entry covers that one number and its refs cite exactly one constant or immediate, and the two don't read the same. The value proposed is the encoding of that number nearest the model's. Moss Hop's check makes `levels.main.kill_below` 200 in a copy, and derivation proposes `PIT_Y`'s 208 (`games/mosshop/mosshop.s:37`) and asks.

### The 6502 reader and evaluator

`gp_derive::asm` parses 6502 source in the ca65 (Moss Hop, Mega Man 2), asm6f (Super Mario Bros. 2) and x816 (Super Mario Bros.) dialects far enough to read facts: labels (global, `@local`, ca65's `:`) and constants, `.db`/`.byte`/`.dw`/`.word` rows, and instructions with their addressing modes, with expressions over constants (`<` and `>`, `+ - * / & | ^ << >>`). It reads every file a pin lists, so a constant defined in one file (`include/constants.inc`) resolves in another, and the reader needs no assembler and no ROM.

`gp_derive::eval` is a constant-propagation pass over a routine in source form: A, X, Y, the carry and the bytes the routine stores, through loads, stores, transfers, increments, shifts, `adc`/`sbc` with a known carry, `and`/`ora`/`eor`, and loads from a data table at a known index. It follows branches inside the window it reads: a label is entered with what every branch and fall-through into it agrees on, and with nothing known when code outside the window refers to it. The window starts at the nearest label and extends back while every reference to that label is inside it. A `jsr` forgets the registers. Mega Man 2's weapon energies of 28 are read this way: the cited `game_init_refill` routine (`src/bank0E_game_engine.asm:187`) stores a constant through a loop. The pass reads source rather than running the gp-nes CPU core on a scratch state because Super Mario Bros.' labels file has no routine addresses (docs/formats/labels.md) and source works with no ROM; running the core is a row below.

### The data deriver

With `--assets DIR` (a directory `gp assets lift` wrote), each asset map entry's `label` names its lifted file, so a ref to `BoundBoxCtrlData` reads the lifted `bounding_boxes` rows even when the disassembly isn't downloaded. The slots' `from` and `kind` are checked against the map. A generator script named in an evidence ref or note is run as `python3 -I -B <script> DIR` and its TOML output is compared fact by fact; Super Mario Bros.' `scripts/model/smb-levels.py` re-derives 2,042 of its level facts this way and Super Mario Bros. 2's `scripts/model/smb2-levels.py` 172.

### How capture finds each class for any game

| Class | For a new game |
|---|---|
| (a) | A pinned disassembly (or the game's own source) and evidence refs to its lines. The reader and evaluator are dialect-level, not game-level; a new dialect is a few line forms in `gp_derive::asm` |
| (b) | The game's asset map with `label`s, lifted from the person's ROM, and any generator script the model's evidence names |
| (c) | Store recordings and a state mapping; the feel measurement uses the mapping's slots for enemy kinds and the mapping's player fields when the tracker finds no player |
| (d) | Nothing more: built from what (a) to (c) read |

## Donkey Kong

`gp model derive games/donkey-kong --assets LIFTED --recording dk.clear-25m.001` on 2026-11-12: of 5290 facts, 456 hand-written, 231 derived, all from data (the lifted tables and asset slots), 231 agree, none differs, none new, none contradicted; 225 more could come from evidence. Donkey Kong has no disassembly in the games repo, so (a) reads nothing; game-printer's derive on its template gave the same score.

## Limits

| Limit | Default until it bites |
|---|---|
| A fact's candidates are every number its evidence entry cites, so an entry covering many facts gives chance matches | The confidence rules above; per-fact refs are a row in docs/open-questions.md |
| The evaluator reads straight-line code and branches within one window; a value set in one routine and used in another isn't followed | A row in docs/open-questions.md |
| A difference is found only where an entry cites one number for one fact | Recordings and generator scripts find the others |
| Comment numbers are a person's word, read at lower confidence | They count as (a) |

## What changed from game-printer

- game-printer's pinned downloads (pinned.rs: the Super Mario Bros., Mega Man 2 and Super Mario Bros. 2 disassemblies) went with those games. A ref `in` a pinned download is listed in the notes as not read.
- A model in a game's package (a folder with `package.toml`) reads its refs and its asset map from the package, where game-printer read them from the repo; `--root` still overrides.
- `--metrics` is gone with the run recorder.

