# gp author: the authoring API prototype

`gp author` is a prototype of an authoring API: a proposer turns a short request ("a floatier jump") and a base template into an edit of the base model's mechanic parameters, gp's own deterministic checks verify it, a failed check's words become the next proposal's feedback, and the whole exchange is written as one log row, the shape training data would later take. It came from game-printer (author.rs) with its flags, checks and log row; the research behind it is game-printer's docs/research/authoring-api.md.

```text
gp author --prompt TEXT --base DIR --dry-run [--target-bits BITS] [--attempts N] [--log FILE] [--assets DIR]
gp author --base templates/mosshop --dry-run --prompt "make the jump floatier" --assets ~/.gameprinter/work/assets/mosshop
```

| Part | What it does |
|---|---|
| `Proposer` | A trait: given the request, the base model and every earlier failure, return a proposal or none. An LLM proposer would implement it |
| `Scripted` | The proposer built: request words pick edits, tried in order. "float" lowers the player's jump gravity to 3/4, then 1/2; "fast" raises the run speed by 1, then 2; "easy" makes the first enemy's patrol step 2 then 4 times slower; "hard" makes it step every frame. The paths come from the base model's roles (the entity whose role is `player`, the first whose role is `enemy`), so the table works on any model with those parts |
| Checks | Schema (the edit applied as a profile switch, so only mechanic parameters change and the model is revalidated), mechanics (the catalogue records), public (the template's tier and source only), reach and playthrough on every level, difficulty against `--target-bits` (within a quarter), feel (the measure the words named), and conformance reported as skipped |
| Loop | Up to `--attempts` (3) proposals; the first that passes every check is kept; a failure's words are the next proposal's feedback |
| Log | One JSON line a request in `--log` (default `$GP_HOME/author/log.jsonl`), with the request, the base's measures, each attempt's proposal, checks, measures and model hash, the kept attempt, empty `edits`, `rating` and `consent` for later, and the gp version and build. Its `id` is the hash of the row. Every check is deterministic, so the same request on the same build gives the same row |

Only `--dry-run` is built: it writes no variant, only the log row. It exits 1 when no proposal passed.

The base is a folder holding model.toml (and template.toml for the public check), and `--assets` its lifted assets (default `$GP_HOME/work/assets/<model.source>`): `gp assets lift ROM -o DIR --map assets/mosshop.toml` for Moss Hop. The searches run the model in this process with the hooks gp-exts registers for it, as game-printer's did; a model whose hooks live only in its game's module runs on its components alone here.

What changed from game-printer: the conformance check's note. It compared the interpreter with a Bevy build (`gp gen conform`), and gameprinter dropped the Bevy generator, so the note says so; the check is skipped either way.

On 2026-11-12, old binary against new on the same lifted Moss Hop, for "make the jump floatier", "easier" and "faster run": the log rows are equal but for the conformance note and so the row id and build. The test, `crates/gp-cli/tests/author.rs`, builds Moss Hop with cc65, checks that the floatier jump passes every check and stays in the air longer than the base, that two runs log the same row, and that an unreachable target fails both attempts with "too easy" as the feedback.
