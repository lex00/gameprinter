# Watching stage 0

`gp watch start NAME ROM` runs stage 0 on a ROM and writes what it finds, as it finds it, to a watch session that a person's page, `gp watch events` and an agent all follow. The levels are docs/research/rom-event-model.md's. Brought from game-printer with its commands, its session folder and its event format; the routes that served it from `gp serve` are on the mechanic servers now.

## What a session does

| Step | Events |
|---|---|
| Opens the session | `watch.start`: the ROM's SHA-256, mapper and PRG size, the start clips by recipe hash, the search's options |
| Reads the ROM alone (the static harvest) | `snapshot`: `static-0.json` and `unread-0.json` with their BLAKE3 hashes, routines read, and the places the reader stopped |
| Runs each start clip (stage 0's own starts, `gp_explore::presses::start_set`, then `--start`'s clips) with L2, frame by frame | `new-code`, `bank` (the first code run in a switched bank), `edge`, `pointer` (a new value a `jmp (ptr)` went through), and `stop.reached` when a run takes a stop's transfer to where it goes. Only what the session hadn't seen is written |
| Reads the ROM again with the runs | `fact` for each routine read now that wasn't, and a second `snapshot` (`static.json`, `unread.json`) |
| Searches as `gp explore` does (`--aim`, `--target`, `--budget`, `--seed`) | `explore.progress` every tenth candidate and at each clip kept; `clip.kept` with the clip's presses and recipe, then that clip's novelty frame by frame |
| Reads behaviour from the start and kept clips' runs (gp-gaps' detectors) | `behaviour`: sound interfaces, background cell kinds and their writers, counters and flags |
| Ends | `fact` for what the kept clips let the reader read; `watch.end` with routines read and the targets reached |

Every event a run gave names the run's recipe (`run`) and the frame. The session keeps each recipe as `recipes/HASH.presses`, so a reader can run it again for L3. The file holds no wall time: the same ROM and options give the same file, byte for byte, for any `--jobs`, and the same file game-printer's `gp watch` wrote.

## The session folder

`$GP_HOME/watch/NAME/` holds `session.json` (the ROM's path and the options), `events.ndjson`, `writer.lock` (held by the one writer while it writes), the four harvests the snapshots name, `recipes/`, `inputs.ndjson` (the page's and the agent's inputs), and `clips/` (the kept clips as a clip set from power-on, as `gp explore --write` writes them). `--replace` starts a session of the same name again.

## The format

One JSON object a line, `gp_events::watch::WatchEvent`:

| Field | Meaning |
|---|---|
| `v` | The format's version, 1 |
| `seq` | 1, 2, 3, ... with no gaps |
| `run`, `frame`, `level` | The recipe, frame and level an event from a run came from |
| `kind`, `data` | The kind and its fields, as the table above lists |

Addresses are named as the static harvest names them: `C27D` in a region nothing switches, `01:8000` in a switched one.

## Following a session

`gp watch events NAME --after SEQ --follow [--kind K]` prints each line as it is written and stops when the writer is done. `gp watch list` lists the sessions.

Both mechanic servers (`gp mechanic start`'s page and `gp mechanic watch`'s) answer these routes, on 127.0.0.1 only, after their host and origin checks (crates/gp-cli/src/watch_serve.rs):

| Route | What |
|---|---|
| `GET /api/watch/sessions` | The sessions: name, ROM, events and whether a writer is writing |
| `GET /api/watch/{name}/events?after=SEQ` | Events after SEQ (default 0), at most 2000 a request |
| `GET /api/watch/{name}/stream` | Server-sent events (`text/event-stream`): `id:` the seq, `data:` the event's line, from `?after=SEQ` or `Last-Event-ID`, for ten minutes before the browser reconnects |
| `GET /api/watch/{name}/file/{file}` | `session.json`, `static.json`, `unread.json`, `static-0.json` or `unread-0.json` |
| `GET /api/watch/{name}/clips/{file}` | The kept clips: `clips.toml` and its start presses |
| `GET, POST /api/watch/{name}/input` | The inputs; POST one (`{v, actor, kind, data}`, kind `view`, `point` or `note`). A point (`{recipe, frame, rect: [x, y, w, h]}`) is answered at once with the sprites under the box and the routine that wrote each |
| `GET /api/watch/{name}/recipes/{hash}` | A recipe as an input log from power-on |
| `GET /api/watch/{name}/rom` | The session's ROM, only when a file with its hash is in `$GP_ROM_DIR` |
| `GET /api/watch/{name}/detail?recipe=HASH&frames=A..B` | L3, every instruction of those frames, made by running the recipe again |

game-printer served the session list at `/api/watch`; that route is the mechanic page's own watch panel here, so the list moved to `/api/watch/sessions`. Every other route has the old path and answer. The stream and `--follow` read the file through `gp_capture::Tail`, which reads only the bytes added since its last look and holds a torn last line until the rest arrives. There is no page view of a session yet: the old one was `gp serve`'s editor.

## Rerun, point and detail

| Command | Writes |
|---|---|
| `gp watch rerun NAME [--watch ADDR] [--hook AT]` | `rerun`, then each watched write (`watch`: address, value, writer) and each hooked instruction about to run (`hook`: its registers), at most 200 of each a recipe, then `rerun.end` |
| `gp watch explore NAME [--aim stops] [--budget N] [--seed S]` | Another search from the start clips, ending with `watch.end` |

`gp watch detail NAME --recipe H --frames A..B` and `gp watch point NAME --recipe H --frame F --rect X,Y,W,H` only read.

Stage 0 reads the NES's 6502 code through gp-events' collector, so `gp watch start` and `gp explore` refuse an arcade ROM in words.
