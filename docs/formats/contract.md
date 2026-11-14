# Fidelity contract format

`fidelity.toml` sits next to a model's `model.toml` and says how closely each part of the model must match the original. This is version 0.2.0 of the format. `crates/gp-contract` parses it and works out which rule governs each fact, and `crates/gp-validate` compiles the rules into checks ([validation](../validation.md)). Its JSON Schema is `schema/contract.json`, printed by `gp model schema contract`. The format came from game-printer unchanged; Donkey Kong's contract is now in its package in the games checkout.

## Example

Moss Hop's contract, `templates/mosshop/fidelity.toml`, in part:

```toml
[contract]
schema = "0.2.0"
model = "mosshop"
default = "metric"

[[rule]]
path = "levels"
level = "structural"
note = "Same tiles, spawns, pit and goal"

[[rule]]
path = "entities.player.jump"
level = "metric"

[rule.tolerance]
airtime = 2
height = "5%"

[[rule]]
path = "entities[role=enemy]"
level = "metric"

[rule.tolerance]
speed = "3%"
```

## Fields

| Key | Meaning |
|---|---|
| `contract.schema` | The format version, `"0.2.0"`. A reader takes the same major and minor version with a patch no newer than its own. This build also reads `"0.1.0"`, since a 0.1 contract is a 0.2 contract with no `when` |
| `contract.model` | The `model.name` of the model it's for. `gp model check` fails on another |
| `contract.default` | The level of any fact no rule names. Required, so a contract always says what happens to the parts it doesn't mention |
| `contract.description` | Optional, for people |
| `[[rule]]` | Any number of rules, in any order |
| `rule.path` | A [model path](model-path.md) or pattern. It must name at least one part of the model, and not the `model` header or the evidence table |
| `rule.level` | `exact`, `metric`, `structural`, `experiential`, `style` or `free` |
| `rule.aspect` | Optional. One aspect of the parts named, such as `arc` or `reach`, so a jump can be Free in arc but Structural in which ledges it reaches. A name in the path grammar |
| `rule.tolerance` | Optional, metric rules only: quantity names to how far each may be from the original's. `"5%"` is relative; `2` or `"0.5"` is in the quantity's own units (pixels, frames). Never negative, and exact like any [fixed-point number](../model.md#numbers) |
| `rule.when` | Optional. A condition over runtime state while which the rule holds ("Exact in world 1"): the run-until condition grammar over state paths, such as `world == 1`, evaluated on each frame of the original's run ([validation](../validation.md#conditions)) |
| `rule.note` | Optional, for people |

`gp validate` checks Exact, Metric and Structural. Experiential and Style parse and resolve, and the facts they govern are listed as unchecked, so a contract written now won't need a format change when checks for them arrive. Free is never checked.

## Corpus bars

`[corpus]` (features `corpus.together` and `corpus.longest_stretch`) says what every clip of the scoring corpus the model is scored against must reach ([scoring corpus](scoring-corpus.md)). `gp corpus score` checks each clip against them and reports the clips below them; the score record holds each clip's result under `bars`.

| Key | Meaning |
|---|---|
| `corpus.together` | A percentage, `"100%"`: each clip's frames on which every measure is within its tolerance, run straight through from its start (free mode, with the original's draws when the state mapping has `score_draws`) |
| `corpus.longest_stretch` | Seconds of the game's own play, `"10s"`, turned into frames at the model's `frame_rate` (601 frames on the NES, 300 on a 30 Hz board): each long run's (a clip of more than 600 frames) longest exact stretch in resync mode from the same start, counting the one the clip's end cuts off. A long run with fewer play frames than that (a game over, a clear scene) isn't held to it |
| `corpus.note` | Optional, for people |

Donkey Kong's (`fidelity.toml` in the games checkout's `donkey-kong` package):

```toml
[corpus]
together = "100%"
longest_stretch = "10s"
```

## Resolution

For one fact (a concrete path from the model, such as `entities.slug.patrol.step`) and one aspect (or none, for the fact as a whole):

1. A rule applies when its path names the fact or a part the fact is in, and the rule has no aspect or the aspect asked about. Levels inherit downward this way: a rule on `entities` covers every fact under it.
2. Among the rules that apply and have no `when`, the winner is the one whose path is most specific ([specificity](model-path.md#specificity)): the nearest part first, then exact names over filters over `[*]`. A rule for the aspect beats one for every aspect at the same specificity.
3. Rules still tied with different levels conflict. `gp model check` reports each conflict; resolution takes the first in the file.
4. With no applying rule, the default applies.
5. Rules with a `when` that apply and are at least as specific as the winner are returned as conditional: each replaces the winner while its condition holds. Two rules with different `when`s don't conflict in the file; if both hold on the same frame with different levels, `gp validate` fails the check there.

Tolerances inherit too. A fact at the metric level gets each tolerance from the most specific applying metric rule that sets it, so a rule on `entities[role=enemy]` can set `speed` and `timing`, and a rule on `entities.slug` can tighten `speed` alone.

| Fact | Moss Hop resolves it to |
|---|---|
| `levels.main.kill_below` | structural, by the rule on `levels` |
| `entities.player.jump.gravity` | metric with `height` 5% and `airtime` 2, by the rule on `entities.player.jump` |
| `entities.slug.patrol.step` | metric with `speed` 3%, by the filter rule |
| `entities.slug.sprite.palette` | style, by the rule on `entities[*].sprite`, which is nearer than the filter rule |
| `entities.player.hitbox.w` | metric, the default |

`gp model check` prints how many facts resolve to each level.

## What `gp model check` reports

| Problem | Example |
|---|---|
| A path that doesn't parse | `entities[` |
| A path that names nothing in the model | `entities.ghost` |
| A path into the header or the evidence | `model.name` |
| Tolerances on a rule that isn't metric | `level = "free"` with `tolerance` |
| Conflicting rules | `entities.slug` structural and `entities.slug` metric |
| A `when` that isn't a condition | `world 1` |
| A `[corpus]` bar that isn't a percentage or seconds | `together = "all"` |
| The wrong model, or a format version this build can't read | `model = "huge"` |

Rules no recording exercises show in `gp validate` as unexercised checks.

## Canonical form

`gp model fmt` rewrites `fidelity.toml` in its canonical form, as it does `model.toml`: the keys in a fixed order, tolerances as their own tables. The rules keep their order, since the order breaks ties between conflicting rules. `gp model fmt --check` writes nothing and exits 1 when either file isn't in it.

## Versions

| Version | Change |
|---|---|
| 0.1.0 | First version. `when` is kept as text, reserved for later |
| 0.2.0 (2026-10-28) | `when` is a condition in the run-until grammar over state paths, evaluated per frame on the original ([validation](../validation.md#conditions)), and `gp model check` reports one that doesn't parse. Contracts: none must change, since no 0.1.0 contract used `when`; a 0.1.0 contract still reads, and changing `schema` to `"0.2.0"` is all an update needs. Plugins: a check plugin that reads contracts evaluates `when` this way; nothing else changes |
