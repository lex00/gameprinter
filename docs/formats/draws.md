# Random draws script

> Brought from game-printer with wave 3b of the port. `gp interp draws` (and `--find`) is in gameprinter; `gp clip score` and `gp corpus score` already play a mapping's draws when it sets `score_draws`, and `gp interp compare --draws` runs a lockstep with a script. `gp game new` proposes the sites an idle run of 3600 frames finds into a new NES game's mapping.toml (`--no-draws` leaves the search out). `--find` reads an arcade romset's main Z80 too, an arcade mapping's `[[draws]]` are recorded in the lockstep, and `gp game new` searches an arcade game's idle run as well (wave 6, below).


A random draws script is the original's random draws on one input, in order: each time its code read a random byte where the model's hooks make a random choice. A rebuild given the script takes each draw from it in place of its own generator, so a lockstep clip makes the original's random choices, and everything after a choice (the object's pixels, the player's touches with it) is compared given the original's branch. Without a script a rebuild draws from its seeded generator, so free play is unchanged. The script is a run input: it isn't world state, in the canonical state or in its hash.

`gp interp draws` records one; `gp interp compare --draws FILE` runs a lockstep with it. `gp_sim::draws` holds it.

## Draw sites

The state mapping's `[[draws]]` (feature `draws`) names the original's draw sites:

| Key | Meaning |
|---|---|
| `site` | The site's name, which the hook draws by |
| `pc` | The address of the instruction that reads the random byte |
| `ram` | The random byte; an indexed read (`AND $19,X`) reads the byte its index names, and the recording takes that one |
| `soft` | Optional (feature `draws.soft`), default false: the byte is read only for presentation, such as a sound driver's random pitch. The script lists the site on a `soft SITE` line; a rebuild takes its draw when it asks on the same frame (`ExtCx::draw_soft`, which gives `None` without a script and takes nothing from the generator), and a draw it doesn't take, or one it asks for that the original didn't make, never desyncs the script. `DrawScript::peek` gives the next frame's soft draw without taking it, for a runtime that plays the next frame's sound ahead (`gp_sim::Ext::apu_peek`) |
| `note` | Optional, for people |

Donkey Kong's:

```toml
[[draws]]
site = "ladder"
pc = 0xDB7B
ram = 0x19
note = "The ladder choice: AND $19 with C448's $88; 0 goes down"
```

Donkey Kong's walk sound picks one of two pitches by bit 3 of $18 at $FC62 (the third site in the table below); its site `sound_buzz` is soft.

A hook draws with `ExtCx::draw(site)`, a byte, and tests it as the original's code does: `cx.draw("ladder") & 0x88 == 0`. The odds are then the original's whatever the byte's source.

## Finding draw sites

```text
gp interp draws --find ROM --recording ID [--mapping FILE | MODEL] [--out FILE]
```

`--find` finds a game's random register and its draw sites from a run of the original, with no model, and proposes `[[draws]]` entries for review. It runs the input twice in gp-nes (`crates/gp-cli/src/draws_find.rs`).

The first run finds the register. A RAM byte is random when all of these hold:

| Test | Rules out |
|---|---|
| Written by a shift or rotate in place (`ROR $18`), or by a store of a value an exclusive or, an add or a shift made, on at least half of the frames run | Bytes only loaded and stored |
| At least 64 values at the end of a frame | Flags, the controller's bits |
| No one change from frame to frame on more than half the frames | Counters and timers |
| A change of more than 8 either way on more than half the frames | Positions and speeds |

Adjacent random bytes stepped alike are one register. The report says how it is stepped: in place or by stores, by which routine, in the main loop or an interrupt handler, and how many times a frame.

The second run follows each read of the register. The value read is traced through A, X, Y, the flags and RAM for 64 instructions. A read whose value reaches a conditional branch, or the index of a table read, is a draw site; the entry's note gives the instruction, its routine, the mask (the `AND` that took bits of it) and the branches with how often each was taken. A read followed by a write to the register at the same stack depth is the register's own step and is left out. A read stored in RAM and not tested within 64 instructions (a timer or a speed set at random) is listed apart, for review, and not proposed.

Each proposed site is named by its address (`draw_fc62`), or by the mapping's name when the mapping already has it. With a model or `--mapping`, the report scores the proposal against the mapping's sites: recall is the hand-written sites found, precision the proposed sites the mapping names.

| Game, run | Register | Proposed | Against the hand-written sites |
|---|---|---|---|
| Donkey Kong, run 5658S (`dk5658-s0-zeros`, 5233 frames, 25m to 100m) | $18-$1F, shifted in place by $F4ED in the idle loop, 75 to 369 times a frame | 7 | Recall 3 of 3 (ladder, roll_off, fire_way); precision 3 of 7. The other four are real draws outside the mapping's 25m scope or sound: $E278 and $E574 (routine $E250), $EA38 (routine $EA34) and $FC62 (the sound routine $FBF2, 18 table indexes) |
| Donkey Kong, the user's walk 2 (7072 frames, 25m only) | The same | 3 | Recall 2 of 3; roll_off ($DCB3) isn't read in the run. Precision 2 of 3, the third being the sound's $FC62 |
| Super Mario Bros., the warps run (TASVideos movie 1715M, `happylee-1715M-warps-fm2`, 17867 frames) | $07A7-$07AD, shifted in place by the NMI handler once a frame; no other register | 14 | The mapping has no `[[draws]]`, so the sites were held against SMBDIS.ASM (doppelganger's disassembly, its reads of `PseudoRandomBitReg`): 22 reads besides the step, 17 of them reached in the run. Precision 14 of 14, recall 14 of 17 reached. The other 3 ($C4F1, $C9B8, $CA51: a flying cheep-cheep's speed offset, the podoboo's timer, the hammer bro's jump timer) store the value for later and are listed for review |

The run decides recall: a site the input never reaches can't be found, so find with the longest runs at hand, across every stage the mapping covers.

### On an arcade romset (Z80)

`--find tapper.zip` runs the same two passes on gp-arcade's main Z80 (`crates/gp-cli/src/draws_find_z80.rs`), reading its instruction trace and its access log (each read and write with the instruction that made it) through the Z80 disassembler. The register is looked for in the work RAM (`e000-e7ff`): stepped by a shift in place (`rlc (hl)`) or by a store of A when A was last made by an exclusive or, an add, a subtract, a rotate or `ld a,r`. The value read is followed through every register (as 8-bit units), the flags and memory (by the addresses the log saw); a conditional jump, call or return on a flag it set (`djnz` on B), a memory operand whose address register it reached, or `jp (hl)` through it, is a draw. Block copies and searches that pass over the register (`ldir`, `cpir`) aren't reads. Two things are new for the Z80:

- **The refresh register.** `ld a,r` reads R, which counts instructions run, and Z80 games mix it into their numbers. A byte whose stores feed back its own value mixed with R is a generator the game calls when it wants a number, not every frame, so it needs only many values over the frames it was stepped (at least 8 frames), not a step on half of them. A byte merely set from such a number doesn't feed back, and isn't one.
- **A generator routine's return.** A Z80 generator returns its number in A. When a routine that wrote the register returns, its call is treated as a read: the caller's use of A is followed, and the call (`call $5fbd, the number it returns`) is the draw site.

| Game, run | Register | Proposed |
|---|---|---|
| Tapper (`tapper`), 3600 frames of attract with no input | $E390, stored by $5FBD (`ld a,($e390)`, `rlca`, `xor $2b` on a carry, `ld c,a`, `ld a,r`, `xor c`, `ld ($e390),a`), on 69 of 3600 frames | `call $5fbd` at $5FAE, inside $5FAC (which brings the number below B by subtracting and halving): 84 reads on 68 frames, deciding table indexes at $59FD and $59FF and the loop at $5FB2. Also $CCE1 (`cp (hl)` in $CCAC, read once), for review |
| Tapper, the five-level autoplay (`tapper-five-levels.input`, 10,933 frames) | The same, on 333 frames, 1 to 14 times a frame | The same call at $5FAE: 427 reads on 332 frames, indexing tables at $259D, $25AC, $25B8 and 8 more |

So Tapper has one random generator, a shift register at $E390 mixed with R, and every number the game takes from it goes through $5FAC. Because R counts instructions, a model of Tapper's draws needs the number of instructions run, which only the original gives exactly: the draws should be taken from the original (`[[draws]]` with `score_draws`) rather than computed.

## Text form

```
gp-draws 0
1433 ladder 102
1510 ladder 88
6968 roll_off 143
```

The first line is `gp-draws 0`. Each other line is one draw: the original's frame (counted from 1, the frame the draw happened in), the site and the byte in decimal. A `soft SITE` line, before the draws, names a soft site. `#` starts a comment.

## Taking draws

A script starts at the lockstep's start, the original's frame `--from`: the state loaded there is the rebuild's frame `origin`, and the rebuild's frame `f` stands for the original's `f - origin + from`. Before the start the hooks draw from the generator. After it, a draw takes the frame's next untaken draw at the same site. Within a frame the sites may be drawn in any order, since a game's objects that frame read the same random state whichever reads first (Donkey Kong runs its fires before its barrels, and the model steps bodies in spawn order); one site's draws are taken in the original's order. Anything else is a desync:

| Desync | Meaning |
|---|---|
| `the rebuild drew X on frame F; the original's next draw is Y on frame G` | The original made no more draws at X on frame F |
| `the original drew X on frame F; the rebuild didn't` | A frame ended with a draw of the original's untaken |
| `the rebuild drew X on frame F, after the original's last draw` | The rebuild drew more times than the original |

A desync is a defect in the rebuild, which then draws from its generator again. `gp interp compare` reports the first one with its frame, or how many draws were taken.

The draws on or before the start count as taken, so one script of a whole run serves a start at any of its frames. The first state loaded starts the script; a later one is a resync (`--mode resync`), from which the script goes on: it forgets a desync and takes the original's draws after the frame loaded, as the original goes on from there. Until 2026-11-02 each load started the script over at the original's start frame, so a resync run with `--draws` desynced at its first split and drew from the generator after it.

## Scoring with draws

A state mapping with `score_draws = true` (feature `draws.score`) scores with the original's draws everywhere a clip is scored, with no script file: `gp interp compare` records one from its input when `--draws` isn't given, a clip's score (`gp clip score`, `gp corpus score` and the gate) records the draws over the clip from its start and keeps them beside the clip's tape, and `gp model refine --lockstep` (the queue's refine rounds) records the whole input's once and gives each trial the draws after its start. A run with no start (a whole run from power-on with no `--from`) takes none, since the script needs the original's frame its first load stands for.

## The behaviour check

Scoring with the original's draws holds what follows each choice; `gp interp odds MODEL --clips clips.toml [--rom ROM]` holds the rule that makes the choice. It reads the rule from the model, with nothing written down for it:

1. The model plays each clip from power-on with its own generator (with `--rom`, on the frames the state mapping lines up with the original's). Every draw a hook makes is kept with its frame and site (the script's `calls`).
2. At each draw the frame is run again from the state before it, with the draw set to each of the 256 byte values by a lenient script (one that sets that draw alone and lets the frame's other draws fall back to the hook's own). The bytes that lead to the same state are one branch, and a branch's share of the 256 is the rule's odds for it there. The draw's own branch is the one whose state the free run reached. A site's second draw in one frame isn't set (a script sets a site's first draw of the frame), and is counted as not tested.
3. The decisions at a site with the same branches are pooled. A pool's count of each branch is tested against the rule's odds with a chi-square test, which fails when its p-value is below 0.001. With `--rom` a branch's odds are the share of the original's draws at the site whose bytes lead to it, since a game's generator needn't give the 256 values evenly: Donkey Kong's register ($18-$1F, stepped by the idle loop) doesn't, and against an even split even the original's own draws failed (roll_off p 0.0005, fire_way p 0.0004, 2026-11-03). Without `--rom` the odds are the 256 values' shares. A pool with an expected count under 5 is reported as too few to test. A decision with one branch only (Donkey Kong's barrel at a crowded ladder) is counted apart.
4. With `--rom`, the original's draws over each clip from its start are put in the site's most common branches, and their counts are tested against the model's in that pool (chi-square on the 2 by k table): whether the two games choose with the same odds. Each clip runs as a resync lockstep with the original's draws: every desync at a site (the original drew where the rule draws nothing, or the reverse) fails the site.

The report names each branch by the bits of the byte it reads: `byte & 0x88: 0x00 (64/256) | 0x08,0x80,0x88 (192/256)` is Donkey Kong's ladder choice, down 1 in 4. It exits 1 when a test fails.

### Recording an arcade game's draws

An arcade mapping (`platform = "arcade"`) takes `[[draws]]` like an NES one, and `gp clip score`, `gp corpus score`, the scenarios and `gp interp draws MODEL --rom ROM` record them from gp-arcade's trace and access log on the main Z80 (`record_draws_arcade` in crates/gp-cli/src/lockstep.rs). A Z80 game's random routine returns its number in A, so a site may name the call (`pc = 0x258A`, `call $5fac`, as `--find` proposes it) or the instruction just after it (`pc = 0x258D`); either way the draw is A when control comes back from the routine, which may be a frame later, and the draw is in that frame. A site after a `call nn` that wasn't taken (a conditional call falling through) draws nothing. Any other site draws the byte its instruction reads, as on the NES.

On Tapper with the eleven sites of games branch tapper-2's mapping (each the instruction after a call of $5FAC) and the five-level autoplay: 427 draws (arrival_wait 131, attract_pick 35, bonus_shake 15, customer_wait_start 153, game_start 1, move_picture 8, pushed_drink 43, throw_wait 41), which is every one of the 427 numbers $5FAC took from $5FBD in that run by `--find`'s count. `--find` itself now follows a number up through a routine that passes it on in A, so it proposes the calls of $5FAC ($258A, $4F35) and lists the others that store their number for later ($09DE, $10C6, $1B6C, $213B, $4AFA) for review.
