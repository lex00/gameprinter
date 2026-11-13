# The fidelity score

The fidelity score is one number that says how close a rebuilt game's play is to the original's, worked out from the scores of all the clips it was measured on, where "71 of 185 clips match exactly" counts only one thing. The user asked for it on 2026-10-29: "the score calculated by our judgement overall based on data about how close to the original gameplay the reimpl is". This page came from game-printer, whose score had three parts (behaviour, coverage and picture); gameprinter has the behaviour part only so far, and the full formula is kept below as the one to port.

It is a different thing from the fidelity contract (`fidelity.toml`, [formats/contract.md](formats/contract.md)), which says how close each part of the model must be, where the score says how close the evidence shows it is.

## What gameprinter has

| Where | What |
|---|---|
| `gp corpus score GAME --run NAME` | Prints "On corpus NAME VERSION (HASH): E of N clips exact, fidelity P%, X couldn't be scored. Model HASH." The fidelity is the mean of every clip's frames together (`permille`), a clip that couldn't be scored counting as 0. With `--json` the score record holds it as `results.fidelity_permille`, beside the clips exact, the errors and the [corpus bars](formats/contract.md#corpus-bars) held and met ([scoring corpus](formats/scoring-corpus.md)) |
| `GET /api/capture/{run}/fidelity` in `gp serve` | Behaviour alone: the mean of the scored active clips' frames together, as a score out of 100, with the clips scored and exact. Coverage and picture are 0 and null, the confidence is always `low`, and there is no history or trend. Its version is `serve-behaviour-0`. The code is `fidelity` in `crates/gp-cli/src/serve/background.rs` |
| The editor | `editor/src/Fidelity.tsx` draws the line from the endpoint's JSON, in the shape game-printer's full score had |

Donkey Kong's last gate score record ([evaluation](evaluation.md)) was 111 of 111 clips exact, fidelity 1000 per mille.

`gp capture fidelity` is here as `gp corpus score` on the run's game, with the fidelity line above. Not here yet: game-printer's own `gp capture fidelity RUN [--json]`, its metric `fidelity.<template>.score`, the coverage and picture parts, the confidence, what holds the score down and the history. Everything below is that score as game-printer worked it out (its `crates/gp-cli/src/fidelity.rs` and `serve/fidelity.rs`), which a port should keep to.

## game-printer's formula, version `fidelity 2`

The score has three parts, each from 0 to 100: behaviour, coverage and picture.

```
weighted = (60 * behaviour + 25 * coverage + 15 * picture) / 100
factor   = 1 if coverage >= 50, else coverage / 50
score    = weighted * factor
```

When no clip has a screen score, picture is "not measured" and its 15 goes to behaviour (75 behaviour, 25 coverage).

| Weight | Default | Why |
|---|---|---|
| Behaviour | 60 | The play is what the score is about: the same presses giving the same movement, frame by frame |
| Coverage | 25 | A score from one level says little about the rest of the game |
| Picture | 15 | It's a signal only: the refine tunes no number on it, and it mixes in the HUD and the model's art |
| Knee | 50 | Below half coverage the score is scaled down in proportion, so a game with little evidence can't score high on what little it has. An untested game scores 0 |

### Behaviour

Over the clips scored and able to start, each clip's value is:

| Part | Weight | What |
|---|---|---|
| Frames together | 40 | The clip's `permille` |
| Exact | 20 | All of it if the clip matches exactly, else nothing |
| Late first wrong frame | 25 | The drift frame over the clip's frames: a clip that goes wrong at its end is closer than one wrong from its first frame |
| Measures exact | 15 | Measures exact over measures compared, from the run's scenario cache; a clip the cache has no result for uses its frames together |

The measures come from the cached result with the clip's ID whose headline matches the clip's score (exact, score, frames together and drift frame). When results of several models match, the one with the most measures is taken, then the first by file name.

Clips are weighted so that one start's twenty near-identical clips don't count twenty times. Each clip's weight is half its share of its start and half its share of its mechanic, so behaviour is the mean of the starts' means and the mechanics' means, taken together.

### Coverage

How much of the game the run has evidence for:

| Share | Weight | Of what |
|---|---|---|
| Levels | 35 | The game's levels with a scored clip. The game's levels are those the mapping's slots name (`[[slots.level]]`) and those the run's recordings reach, else the model's |
| Mechanics and numbers | 30 | The player's mechanics and their numbers, as the scenario ladder exercises them (`gp_interp::scenario::ladder`), exercised by a scored clip's mechanic and params |
| Entity kinds | 35 | The model's entity kinds a scored clip measures: the player, each kind with a body paired across the two games in it ([bodies](interpreter.md#bodies-enemies-as-measures), `paired` on the clip's score), and any kind whose measure goes wrong. A start next to an enemy isn't enough on its own |

A clip that can't start is no evidence: it counts neither for nor against. A start the rebuilt game can't begin at has no clips, so its level and its enemies stay uncovered, and the reason is said ("enemies aren't carried over yet (75 starts)").

### Picture

The screen measure's frames the same (`screen` on a clip's score): the characters' and the background's, weighted as behaviour weights the clips, and averaged.

### Confidence

| Level | When |
|---|---|
| low | Fewer than 10 clips, fewer than a quarter of the levels, or no mechanic exercised. The line says "low evidence" |
| high | 30 clips or more, three quarters of the levels, three quarters of the mechanics and numbers, and half the entity kinds |
| medium | Otherwise |

The sentence says what the score is based on: the clips and levels, the player's numbers exercised by two clips or more, and the starts and clips left out.

### What holds it down

Each shortfall is turned into the score points it costs, and the three that cost the most are said in plain words.

| Kind | Grouped by | Says |
|---|---|---|
| Behaviour | The measure that goes wrong first, with the clips' mechanics | "Mario's speed up and down goes wrong (59 clips: 26 in the jump, 23 in the run, 10 other)" |
| Levels | | "8 of 37 levels have no scored clip" |
| Mechanics and numbers | | "4 of 21 Mario numbers aren't exercised by a scored clip" |
| Entity kinds | With the most common reason a start can't begin | "enemies aren't carried over yet (75 starts), so 36 of 37 kinds of thing are in no scored clip" |
| Picture | The layers that differ and what is in the places that differ | "the picture looks different: the background on 30% of frames (185 clips; where it differs: the score in 160)" |

The coverage items also carry the knee's cut, shared in proportion to their shortfalls.

### Determinism

Every sum is in integers (parts per million) and clips are taken in ID order, so the same scores give the same number on every machine, whatever order the files list them in. Nothing reads the clock. game-printer tested the formula on made-up scores, and on Moss Hop: 0 with low confidence before any clip is scored, then high (100, picture 99) once every clip matched, and the same JSON when asked again.

### The history

The run kept one entry per model hash in `fidelity/history.json`: the score, its parts, the clips, and what changed since the entry before ("after fixes to the jump", from the background queue's rounds, else "after a model change"). The model is the one most scored clips were scored on, so a model whose clips haven't been scored again yet doesn't take the old model's number. A new model adds an entry; asking again on the same model brings its entry up to date. It kept the last 50. The trend is the last two: "62 -> 71 after fixes to the jump".

Clips scored on an older model than the working copy's are counted as they stand and reported as `stale`; the background queue scores them again.

## Changing the formula

A change to a weight, a threshold or how a part is worked out bumps the version and adds a line here. Scores of different versions aren't compared: the history is of one version's numbers, so a version change starts the trend again.

| Version | Change |
|---|---|
| `fidelity 1` | First version, in game-printer, 2026-10-29 |
| `fidelity 2` | An entity kind is covered by a paired body in a scored clip, not by a start next to it (game-printer, 2026-10-30) |
| `serve-behaviour-0` | gameprinter's `gp serve` endpoint: behaviour alone, the mean of the scored clips' frames together |
