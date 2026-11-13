# Capture events format

A capture run (game-printer's docs/capture-live.md) writes what happens as an event stream, and takes inputs from people and agents as a second stream. Both are NDJSON (one JSON object a line). Lines are appended and never rewritten. This is format 1. The types are in `crates/gp-capture` and the JSON Schema generated from them is `schema/capture-events.json` (`gp self schemas` writes it, and `--check` fails while it's stale).

Brought from game-printer's docs/formats/capture-events.md. The format and `gp-capture`, which reads and appends it, came whole. What writes a run in gameprinter is less: `gp mcp` appends an agent's inputs (every tool's `run` argument, `capture_input`, `requests`) and reads the events (`capture_events`), and `gp watch` takes the same writer lock for its sessions ([watch.md](../watch.md)), whose events are gp-events' own format. Not here yet: the writers of a run's events, `gp capture start`, `resume` and `watch`, and `gp capture input`; `gp serve`'s routes for a run's events, stream and inputs (its `/api/capture/{run}/background` routes are the background work of the mechanic page's REFINE tab, a different thing); `gp capture schema`, whose job `gp self schemas` does; and the play sessions and point-and-tell page that wrote the kinds below.

Format 0 was the research prototype's (game-printer's docs/research/live-capture.md). Format 1 adds the kinds `row` and `run.state`, the input kinds `unpin` and `tool`, and optional fields, and changes one meaning: `questions.open` no longer counts pinned questions. Readers take inputs of format 0 and 1.

## Where they live

`$GP_HOME/capture/<run>/events.ndjson` and `$GP_HOME/capture/<run>/inputs.ndjson`. A run name is made of letters, digits, `-` and `_`, up to 80 characters. It can't be `cache`, the folder that holds stage outputs shared across runs.

| File | Written by | How |
|---|---|---|
| `events.ndjson` | One writer at a time: game-printer's `gp capture start`, `resume` or `watch` | The writer holds an operating-system lock on `writer.lock` in the run folder, released when it exits or crashes. Each line is checked against the format before it's written |
| `inputs.ndjson` | Anyone: `gp mcp` here; in game-printer also `gp capture input` and `gp serve` (`POST /api/capture/{run}/input`) | Each line in one `write`, so senders in other processes don't interleave within a line |

A reader skips a last line without a newline (a write in progress) and reads it next time. A reader skips a kind it doesn't know.

## Versions

| Change | Version |
|---|---|
| A new optional field, event kind or input kind | Stays 1 |
| A field's meaning changes, a field is removed or becomes required | Goes to 2, with this page saying what readers must change |

## Events

```json
{"v":1,"seq":41,"at_ms":2950,"round":1,"actor":"pipeline","stage":"validate","kind":"stage.end","data":{"status":"fail","ms":1510,"summary":"19 pass, 2 fail, 0 unexercised"}}
```

| Field | Meaning |
|---|---|
| `v` | The format version, 1 |
| `seq` | 1, 2, 3, ... with no gaps. A reader asks for the events after the last `seq` it has; it is also the server-sent events `id` |
| `at_ms` | Wall time since the run started, for a person to read. Nothing compares it, so it's the one part of a run that differs between two runs on the same inputs, and no runtime reads it |
| `round` | The round the event belongs to; 0 for `run.start` |
| `actor` | `pipeline`, `person` or `agent` |
| `stage` | The stage, for stage events and what they found |
| `kind` | One of the kinds below |
| `data` | The kind's fields |

| Kind | Data |
|---|---|
| `run.start` | `run`, `model`, `stages` (the stages each round runs, in order) |
| `round.start` | none (`{}`) |
| `stage.start` | `calls`: the commands it runs |
| `stage.end` | `status` (`ok`, `fail`, `error`, `cached`, `skipped`), `ms`, `summary`; for `cached`, `result` (how the replayed run of the stage ended) and, when it came from another run, `from_run` |
| `item` | `what` (`asset`, `recording`, `fact`, `check`, `label`, `gap`, `frame`, `asset_edit`), `id`, `status`, `text`, `pinned: true` for an item pinned as won't model, and for `asset_edit` optional `data` (its preview files and checks) |
| `metric` | `key` (a scorecard metric name where one exists, game-printer's docs/scorecard.md), `value`, `prev`; `budget` for a tracked speed metric |
| `question` | `id`, `text`, `path`, `candidates` (each `value`, `from`, and `refs`, the evidence refs it was read at) |
| `input` | `kind`, `status` (`applied`, `refused`, `noted`), `summary`, and `input` as sent; the event's actor is the sender's |
| `row` | `table` (`refine`, `catalog`), `index` (from 1, going on across rounds), `cells` (the row's numbers, strings and booleans) |
| `round.end` | `summary`, `ms`, `failed`, `metrics` (every number the round reported), `deltas` (those that changed, each `{value, prev}`), `open` (open questions and gaps, less the pinned ones) |
| `run.state` | `state` (`running`, `debounce`, `waiting`, `steady`, `stopped`) and `reason`, from the watcher |

Numbers a stage reports that changed since the last value seen are `metric` events as the stage ends, so a panel's numbers move during a round, not only at its end. `questions.open` and `gaps.open` are counted by the round, leaving out pins.

A `check` item's optional `data` has the failing check's `part` and `quantity`, its `kind` and `against`. It also has `expected` (the original's value) and `actual` (the model's), with `frame` and `from` (the baseline it failed on). A `gap` item's optional `data` has the gap's `place`, `recording`, `samples` (frames it was seen on), and for a sprite its `size`, `palette` and `tiles`. game-printer's workspace phrased both in game terms (its docs/inspector.md, "The game view"). Added 2026-10-29; the format stays 1.

### Play session kinds

A play session (game-printer's docs/show.md, "show the AI what it's missing") is a run folder too, and writes these kinds into the same stream. `gp serve` checks each line against the format before it writes it. Added 2026-10-29; the format stays 1, since they're new kinds.

| Kind | Actor | Data |
|---|---|---|
| `play.start` | person | `run`, `rom` (headerless SHA-256), `source` (or null), `name`, `template` (or null), `store`, `real_store`, `notes` |
| `play.mark` | person | `id`, `from`, `to`, `note`, `by` (`key` or `timeline`), optional `annotation` |
| `play.stop` | person | `frames`, `digest` (the page's frame hash digest), `marks` |
| `play.recorded` | pipeline | `id` (the recording), `frames`, `lag_frames`, `store`, `real_store`, `digest`, `recording_digest`, `digest_matches`, `provenance` (`human-play`) |
| `play.annotated` | pipeline | `mark`, `annotation`, `recording`, `from`, `to` |
| `play.error` | pipeline | `message` |
| `show.start` | pipeline | `marks`, `template` |
| `show.mark` | pipeline | `mark`, `from`, `to`, `note`, `summary`, `missing` (lines), `model`, `original` (each side's findings as `gp show explain` gives them), `recording` |
| `show.end` | pipeline | `ms`, `marks`, `has_code`, `no_code_why`, `model_error` |
| `show.preview` | pipeline | `status` (`building`, `ok`, `skipped`, `failed`), and `url` and `run` when ok, `why` otherwise |
| `show.skipped` | pipeline | `why` |
| `show.error` | pipeline | `message` |

### Point and tell kinds

A person's point-and-tell request and the replies to it (game-printer's docs/point-and-tell.md). Each is first an input (`request`, `reply`) and then an event of its own kind. The event is written by the run's watcher when it applies the input, or by the sender when no writer holds the run (taking the writer lock for the one line); both look for it first, so it's written once. Added 2026-10-29. These are new kinds, so the format stays 1.

| Kind | Actor | Data |
|---|---|---|
| `request` | person | `id` (its number, from 1), `frame`, `recording` (by name), `choice` (`missing`, `position`, `look`, `sound`, `react`, or none), `text`, `label` (what was pointed at, "a brick"), `context` (the input's whole `data`: the box, the things and each game's world box) |
| `request.reply` | agent | `request`, `reply` (its number among the request's replies, from 1), `status` (`explained`, `proposed`, `needs_mechanic`, `waiting`), `text`, `lines`, `data` (both sides, `piece`, `would_build`, `proposal`) |

## Inputs

```json
{"v":1,"actor":"person","kind":"answer","question":"derive:levels.main.kill_below","choice":2}
```

| Field | Meaning |
|---|---|
| `v` | 0 or 1 |
| `actor` | `person` or `agent` |
| `kind` | `answer` (with `question` and `choice`, from 1), `pin` and `unpin` (with `path`), `note` (with `text`), `tool` (with `tool`, and `args` and `result` cut to a line), `asset_edit` (an agent's, with `item` and `data`), `sound_edit` (an agent's, with `item` `sound:<seq>`, `text` and `data` holding the edit number and its before and after WAV paths), `accept` and `reject` (with `item`), `request` (a person's, with `item` `request:<n>`, `text` and `data`: frame, recording, choice, label, box and the things there), `reply` (an agent's, with the request's `item`, `text` and `data`: status, lines, explanation, and a `proposal` item or `would_build`) |
| `question`, `choice`, `path`, `text`, `tool`, `args`, `result`, `item`, `data` | As the kind needs; no other fields are taken |

An agent's sprite edit (game-printer's docs/sprite-editing.md; the tools are in [mcp.md](../mcp.md)) is an `asset_edit` input: `item` is `<copy>:<seq>`, `text` a line for the log, and `data` its status (`proposed`, `placed`, `accepted`, `rejected`, `undone`), entity, slot, frames, ops, previews, shots, checks and `server` (where the agent's MCP server keeps the copy). `assets_edit` appends one for every edit and decision (undo and redo included) when it names the run. A person's `accept` or `reject` with the same `item` is applied when it's sent, by game-printer's `gp serve` and `gp capture input`; the writer echoes it as an `input` event and each `asset_edit` as an `item` event with the edit's status then. None of the three changes what a round finds. Added 2026-10-29; the format stays 1.

A `request` sent without an `item` is numbered as it is posted (`gp_capture::requests::post`, which game-printer's `gp serve` called and `gp mcp`'s `requests` calls); a request's state (`open` until a reply, then the last reply's status, then `accepted` or `rejected` once the person decides on a reply's proposal) is folded from the inputs alone. Added 2026-10-29; the format stays 1.

Accepting or rejecting a derived or measured fact is an answer to its question: for a derive question, candidate 1 is the model's value and 2 the derived one. A pin's `path` is a question ID, a model path or a gap ID; it covers a question whose ID or path it is, and an item whose ID it is.

The writer applies inputs in order. Answers, pins and unpins change what a round finds, so they're applied at the start of the next round (the watcher starts one after its debounce). Notes and tool calls change nothing a round finds, so a watcher echoes them as soon as they arrive. Each is echoed as an `input` event with its sender as the actor.

## Transport

Not here yet: game-printer's `gp serve` served the stream in three ways, listed with its other endpoints in its docs/inspector.md.

| Request | Answers |
|---|---|
| `GET /api/capture/{run}/events?after=SEQ` | The events after `SEQ`, at most 2,000 |
| `GET /api/capture/{run}/stream?after=SEQ` | Server-sent events (`text/event-stream`): `id:` is the event's `seq`, `data:` the event's line; a reconnecting browser's `Last-Event-ID` takes the place of `after`. A stream lasts ten minutes and sends a comment every 15 s while idle |
| `POST /api/capture/{run}/input` | Appends an input after checking it (422 when it doesn't say enough) |

`gp mcp`'s `capture_events` tool answers the same events after a sequence number, with the questions still open ([mcp.md](../mcp.md)). A watch session serves its own events the same way (`gp watch events`, [watch.md](../watch.md)).
