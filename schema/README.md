# Schemas

JSON Schemas for the file formats Game Printer reads, one file per format. Each is generated from the Rust type that reads the format (with schemars) by `gp self schemas`, which `just docs` runs. Don't edit them by hand. The list of formats is `FORMATS` in `crates/gp-cli/src/schemas.rs`, and a test there compares this folder with it on every run: a stale or missing file, or a JSON file no format makes, fails it. A file the parser takes fits its schema; the parser is stricter, refusing inexact decimals, numbers out of range, references between parts that don't hold and facts without evidence (`gp model check`).

A file name has no version in it. The `$id` inside names the format and its version, which bumps only for a breaking change. An optional part added since is a named feature, not a bump (docs/model.md, "Versions"). `gp model schema core|platformer|topdown|model|contract|slots` prints one, as in game-printer.

| File | Format |
|---|---|
| `model-core.json` | Model core, `model.toml` (fixed core) |
| `platformer.json` | Platformer extension |
| `topdown.json` | Top-down extension |
| `model-platformer.json` | The core with the platformer extension merged in, for typed editing of `model.toml` (docs/model.md, "Editing with types") |
| `contract.json` | Fidelity contract, `fidelity.toml` (docs/formats/contract.md) |
| `mapping.json` | State mapping, `mapping.toml` (docs/formats/state-mapping.md) |
| `profiles.json` | Physics profiles, `profiles.toml` (docs/formats/profiles.md) |
| `corpus.json` | Corpus plan, a game's `corpus.toml` at format 1.0.0 (docs/formats/scoring-corpus.md) |
| `chunks.json` | Chunk catalog, the `chunks.json` `gp chunks` writes (crates/gp-chunks) |
| `clips.json` | Clip set, `clips.toml` and `aimed.toml` (docs/formats/clips.md) |
| `scoring-corpus.json` | Scoring corpus manifest, the older `corpus.toml` beside a clip set |
| `package.json` | A package, `package.toml` (docs/packages.md). The known keys and their types; a key not listed is allowed, as the readers allow it |
| `roms.json` | A game's ROM sources, `roms.toml` (docs/sources.md) |
| `clip-corpus.json` | Clip corpus, a run's `corpus/corpus.json` |
| `recording.json` | Recording manifest, a recording's `manifest.json` in the store (docs/formats/recording.md) |
| `slots.json` | Asset slot state, `slots.toml` |
| `theme.json` | A theme, `theme.toml` (the older theme format; a large-art theme's `look.txt` and `art.txt` are lines, not TOML) |
| `theme-stack.json` | A theme stack, `stack.toml` |
| `capture-events.json` | A capture run's event and input lines (docs/formats/capture-events.md) |
| `rom-dataset.json` | A ROM dataset's rows, one schema a table (docs/formats/rom-dataset.md) |
| `watch-events.json` | A watch session's event lines |
| `image-config.json` | An image format config layer (docs/formats/image-config.md) |

game-printer also had `loop.json` (`gp loop`, dropped). `corpus.json`, `package.json` and `roms.json` are new here.

## Typed editing

`.taplo.toml` at the repo root maps files to these schemas by name for Taplo, Even Better TOML (VS Code) and Zed: model, contract, mapping, profiles, corpus, clips, package, roms, theme, stack and slots files. `crates/gp-cli/tests/schema_mapping.rs` validates every file the rules cover in templates/ and games/, and in the games checkout beside the repo when there is one.

A games checkout opened in an editor on its own needs a `.taplo.toml` of its own pointing back here. With gameprinter beside it, this one works:

```toml
[[rule]]
include = ["**/model.toml"]
[rule.schema]
path = "../gameprinter/schema/model-platformer.json"

[[rule]]
include = ["**/mapping.toml"]
[rule.schema]
path = "../gameprinter/schema/mapping.json"

[[rule]]
include = ["**/corpus.toml"]
[rule.schema]
path = "../gameprinter/schema/corpus.json"

[[rule]]
include = ["**/package.toml"]
[rule.schema]
path = "../gameprinter/schema/package.json"

[[rule]]
include = ["**/roms.toml"]
[rule.schema]
path = "../gameprinter/schema/roms.json"

[[rule]]
include = ["**/fidelity.toml"]
[rule.schema]
path = "../gameprinter/schema/contract.json"

[[rule]]
include = ["**/profiles.toml"]
[rule.schema]
path = "../gameprinter/schema/profiles.json"
```

Not here yet: a schema for `observe.toml`, `probes.toml`, `labels.toml` and `template.toml`, which are read as plain tables, and for `triage.toml`, whose reader is on the quest-triage branch.
