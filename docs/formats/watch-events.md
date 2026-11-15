# Watch events format

A watch session ([watch.md](../watch.md)) writes what it finds to `$GP_HOME/watch/NAME/events.ndjson`, one JSON object a line, appended by one writer and never rewritten. The types are `gp_events::watch` (`crates/gp-events/src/watch.rs`), and the JSON Schema generated from them is `schema/watch-events.json` (`gp self schemas` writes it, and `--check` fails while it's stale). This is format 2.

The page's and the agent's inputs, `inputs.ndjson` in the same folder (`WatchInput`), are a format of their own and stay at 1.

## Versions

| Version | What changed |
|---|---|
| 1 | game-printer's format, brought whole. A `hook` event held the 6502's registers, `a`, `x`, `y`, `s` and `p`, with no tag |
| 2 | 2026-11-14. A `hook` event's registers are its CPU's, tagged by `cpu`: `6502` or `z80`. Every other kind is as in format 1 |

A reader takes both. A `hook` event with no `cpu` is format 1's and reads as the 6502's, so a session written before the change reads as it did. A session that was written in format 1 and then rerun has format 1 lines then format 2 lines; each line's `v` says which it is. A `cpu` other than `6502` or `z80`, or a register missing for the CPU named, is an error.

| Change | Version |
|---|---|
| A new optional field or event kind | Stays 2 |
| A field's meaning changes, a field is removed or becomes required | Goes to 3, with this page saying what readers must change |

## A line

```json
{"v":2,"seq":7,"run":"ab12","frame":489,"level":2,"kind":"stop.reached","data":{"stop":"followed, not read@C27D","routine":"sub_01_8000"}}
```

| Field | Meaning |
|---|---|
| `v` | The format's version, 2 (1 on lines written before 2026-11-14) |
| `seq` | 1, 2, 3, ... with no gaps |
| `run`, `frame`, `level` | The recipe (by hash), frame and level an event from a run came from |
| `kind`, `data` | The kind and its fields; [watch.md](../watch.md) lists the kinds and when each is written |

## The hook event

`gp watch rerun NAME --hook AT` (the agent's `watch_hook`) writes one for each time a hooked instruction is about to run, at most 200 a hook a recipe. `at` names the instruction: as the static harvest names it on the NES (`C27D`, `01:8000`), as a main-CPU address on an arcade board (`0158`). The registers are as they were before the instruction ran.

```json
{"v":2,"seq":40,"run":"ab12","frame":610,"level":2,"kind":"hook","data":{"at":"01:8000","cpu":"6502","a":1,"x":2,"y":3,"s":253,"p":36}}
{"v":2,"seq":41,"run":"cd34","frame":95,"level":2,"kind":"hook","data":{"at":"0158","cpu":"z80","af":4660,"bc":1,"de":2,"hl":57344,"ix":3,"iy":4,"sp":59376}}
{"v":1,"seq":40,"run":"ab12","frame":610,"level":2,"kind":"hook","data":{"at":"01:8000","a":1,"x":2,"y":3,"s":253,"p":36}}
```

| `cpu` | Fields |
|---|---|
| `6502` | `a`, `x`, `y`, `s` (the stack pointer) and `p` (the status flags), each a byte |
| `z80` | `af`, `bc`, `de`, `hl`, `ix`, `iy` and `sp`, each a 16-bit pair: `af` holds A in its high byte and the flags in its low byte. The main board's Z80 only; the sound board's isn't hooked |

The last line above is format 1's, and reads the same as the first.
