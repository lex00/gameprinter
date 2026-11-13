# Finding play no run has made yet

`gp localize GAME` looks for behaviour none of a game's runs has reached and sends a search to it. It came from game-printer on 2026-11-13, where it was built to find Super Mario Bros.' hidden 1-up block with the tools told nothing about it (game-printer's research report on easter eggs as a probe test).

| Step | What it does |
|---|---|
| 1. One-way branches | Every clip of the game's mechanic sessions (except harvest and jumpathon ones) and every blind farm run of its ROM is replayed while each conditional branch is watched. A branch that ran in the level but only ever went one way is a lead |
| 2. Guards | The frame each lead first ran is replayed with the writer slicer (`gp_nes::slice`), and the instruction that set the branch's flag is sliced back to the RAM bytes and constants it came from. When the flag came from comparing one RAM byte with a constant, the values that would send the branch the other way are worked out |
| 3. Places | A guard that read the level grid (the mapping's `[cells]`) is checked against the grid as the runs saw it: a cell that holds a value the other way needs is a place, and where the cell sat from the player each time the guard ran says how to meet it |
| 4. Probes | Go-Explore from a savestate before the place, with the moves ordered by the decision model (`--decide`), at random, or not aimed, scored by the distance to meeting the cell as the guard does; it stops when the branch goes the other way. `--no-probe` stops before this step |

The game is its package in the games checkout: the level key is its `observe.toml` `[level]` (`--level`, Donkey Kong's `main` by default), the grid its `mapping.toml` `[cells]`, the player's bytes the ROM dataset's or the mapping's `entities.player[0].x` and `.y`. With `--rom` and no game, blind discovery finds the player and the whole run is one level. `--poke` adds a sweep that puts the player beside each grid cell along the furthest run; `--collect`, `--check-log` and `--grade` are the grader's parts (a run checked for the branch going the other way, against a store recording).

It reads the NES's 6502 and refuses an arcade romset in words. An arcade version needs each conditional jump's direction from gp-arcade's instruction trace (`crates/gp-cli/src/covered.rs` reads them), a slice back from a jump's flags on the main Z80 (`crates/gp-cli/src/element_z80.rs` slices back from a write), and a level grid in the mapping, which Tapper's hasn't.
