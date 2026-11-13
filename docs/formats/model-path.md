# Model paths

A model path names a part of a game model: one fact, a table of them, or, as a pattern, many parts at once. Contract rules, evidence entries, profile switches and model diffs use it, and the interpreter names its state with it (`gp query`, run-until conditions). The parser is `gp_model::path` in `crates/gp-model/src/path.rs`. The [model docs](../model.md) say what the parts of a model are. This page came from game-printer; the grammar is unchanged, and the runtime protocol's own page isn't in this repo yet.

## Grammar

```text
path     := segment ("." segment)*
segment  := name ("[" index "]")?
name     := [A-Za-z_][A-Za-z0-9_]*
index    := "*" | number (".." number)? | name "=" value
value    := number | name | string
string   := '"' (any character but '"' and '\', or '\"' or '\\')* '"'
number   := "-"? (digits | "0x" hex | "$" hex)    ("_" may separate digits)
```

Spaces are allowed inside brackets and nowhere else. A path has at most 64 segments (`MAX_SEGMENTS`), and a range must end after it starts.

| Path | Names |
|---|---|
| `entities.player.jump.gravity` | One fact: the player's gravity |
| `entities.player.jump` | The player's whole jump component, every fact in it |
| `levels.main.spawn[1]` | The second spawn of the level `main` (positions count from 0) |
| `levels.main.spawn[0..2]` | The first two spawns: a half-open range |
| `entities[*].sprite` | Every entity's sprite |
| `entities[role=enemy]` | Every entity whose `role` is `enemy` |
| `levels.main.spawn[entity=slug]` | Every spawn of the entity `slug` |
| `a[name="two words"]` | A filter on a value that isn't a bare name |

On a list, an index picks the items at a position or in a range. `[*]` picks all of them, and a filter the ones it keeps. Tables have no positions, so on a table only `[*]` and filters pick entries, and a position or range names nothing. A filter keeps the items or entries that are tables with the field set to the value: a number matches an integer, a name or quoted string matches a string, and a bare `true` or `false` matches a boolean. Fractions are strings in a model, so a filter on one quotes it: `entities[gravity="0.25"]`.

## Concrete paths and patterns

A path with no `[*]`, range or filter is concrete: it names at most one part. Diffs and problem reports always print concrete paths, with positions in decimal (`levels.main.spawn[1].params.patrol_end`), and every path they print parses back to the part it names. The exception is the evidence table, whose keys are paths themselves: reports label an entry `evidence."<key>"`.

A pattern names every part it resolves to in a model. Resolving walks tables in key order and lists in position order, so the result is the same on every run.

## Specificity

Contract rules and evidence entries both pick "the most specific match" for a fact. A path's specificity is two numbers, compared in order:

1. Its steps: one for each segment and one for each index. The longer path names a part nearer the fact.
2. Its weight: 3 for each name or position, 2 for each filter or range, 1 for each `[*]`.

So `entities.slug` beats `entities[role=enemy]`, which beats `entities[*]`, and `entities[*].contact` beats `entities.slug` because it names a part nearer to `entities.slug.contact.bounce`.

## Runtime selectors

The selectors the NES runtime takes (`ram[0x0770]`, `ram[0x0700..0x0710]`, `ram[*]`, `cpu.pc`, as `gp query` and `gp run-until` read them in [recordings](../recording.md)) are the subset of this grammar without filters. They parse to the same segments in both parsers, which the test `runtime_selectors_are_a_subset` in `crates/gp-model/src/path.rs` checks for every selector form, and `gp-runtime` refuses filters. The two parsers are separate code: `gp-runtime` doesn't depend on `gp-model`.

Model paths print positions in decimal. Runtime selector results print RAM indexes as `0x` and four hex digits, since they're addresses.

## What names exist

The grammar doesn't fix the names. For a model they're its TOML keys, and the schema requires every key a path can reach to be a name in the grammar: letters, digits and `_`. The evidence table's keys are themselves paths, so they're quoted in TOML (`[evidence."entities.player.jump"]`) and paths don't reach into the evidence table or the `model` header.

A model run's state is named under the same tree ([the interpreter](../interpreter.md), "State by model path"), so a condition such as `entities.player[0].on_ground` reads the same way a contract rule names the player.
