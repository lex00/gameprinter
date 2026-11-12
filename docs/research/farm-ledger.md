# Blind farming ledger

One row per change to blind farming (`gp farm --blind`, crates/gp-cli/src/farm/blind.rs), scored by `gp farm bench` on the train games in bench/farm-blind.toml. The score is the levels finished over the train games, then the median rounds a level. A change is kept only when the train score improves (more levels, or the same levels in fewer rounds) and no game loses a level; otherwise it is reverted. The held-out games (`--held-out`) are never run by the loop: a fresh agent runs them when the person asks.

`gp farm bench --ledger "CHANGE"` appends a row with the date, the commit and the score; the last column (kept or reverted) is filled in by hand. game-printer's ledger (docs/research/farm-ledger.md there) holds the rows before the port, on its own bench of other games.

| Date | Commit | Change | Train score | Kept |
|---|---|---|---|---|
