# Profiles format

A profiles file names other ways to play a model as switches: each one a named fix or assist a player turns on, such as Donkey Kong's `smooth_walk` (Mario moves by his exact speed, not in whole pixels on a 15-frame counter) or Moss Hop's `slow_slugs`. Each switch is a set of overrides of the model's mechanic parameters and options, and any of them combine. A preset names a set of switches, such as `normal` for every engine fix. The file is an optional `profiles.toml` beside `model.toml`, and `crates/gp-model/src/profile.rs` is the code that reads and applies it. What a profile is for, and where each tool lets you choose one, is in [the model's "Profiles"](../model.md#profiles). This page describes format `0.2.0` of the file. It came from game-printer; the format is the same, and the example is now Donkey Kong's.

Its JSON Schema is `schema/profiles.json`, written by `gp self schemas`; `.taplo.toml` maps every `profiles.toml` to it for editors.

The model as written is the profile `retro`, every switch off, which the file can't define. Every comparison and score runs `retro`, and so does the background work.

A game's file has the preset `normal`, the game with no lag and none of the console's quirks, and a switch for frame timing goes in the group `timing`, which `normal` turns on whole. The in-game settings screen lists the file's groups, switches and presets ([the settings screen](../settings.md)).

## Example

This is part of Donkey Kong's file, `profiles.toml` in the games checkout's `donkey-kong` package:

```toml
format = "0.2.0"

[[groups]]
description = "What the console's frame timing decides, off"
name = "timing"
title = "Timing"

[[groups.switches]]
description = "The random numbers step once a frame, not once for each pass of the loop the game waits for the next frame in, as many as the frame's work leaves time for"
name = "rng_per_frame"
off = ["entities.idle.ext_params.idle_loop"]

[[groups]]
description = "The NES engine's quirks off, one at a time"
name = "fixes"
title = "Engine fixes"

[[groups.switches]]
description = "Mario walks and drifts in a jump by his exact speed each frame, not in whole pixels on a 15-frame counter"
name = "smooth_walk"
off = ["entities.player.run.step_cycle", "entities.player.run.air_step_cycle"]

[[groups.switches]]
description = "A jump that lands 16 pixels or more below where it started doesn't kill Mario"
name = "high_landings_survive"

[groups.switches.set]
"entities.player.jump.fatal_drop" = 256

[presets.normal]
description = "No lag and none of the NES engine's quirks: every switch on"
switches = ["rng_per_frame", "smooth_walk", "high_landings_survive"]
```

The real file has 12 switches, and its `normal` turns on all of them.

## Keys

| Key | Type | What it is |
|---|---|---|
| `format` | string | The format version, `0.2.0`. A reader takes the same major and minor with a patch no newer |
| `groups` | list of tables | The switches in groups, in the order an options screen shows them |
| `groups[].name` | string | The group's name: letters, digits and `_`, not starting with a digit |
| `groups[].title` | string | The group's heading on an options screen. Optional; the name without it |
| `groups[].description` | string | What the group is, in plain words. Optional |
| `groups[].switches` | list of tables | The group's switches, at least one |
| `groups[].switches[].name` | string | The switch's name, unique in the file and not a preset's: letters, digits and `_`, not starting with a digit, and not `retro` |
| `groups[].switches[].description` | string | What turning it on does, in plain words. The options screen shows it |
| `groups[].switches[].set` | table | New values by model path. Each is of the type the model has at that path; a fixed-point number may be written as an integer or a decimal string, as in `model.toml` |
| `groups[].switches[].off` | list of strings | Model paths to remove, so each goes back to its default: an option off, a list empty |
| `presets.<name>` | table | A named set of switches. The name follows the switches' rule and isn't `retro` |
| `presets.<name>.description` | string | What the preset is. Optional |
| `presets.<name>.switches` | list of strings | The switches it turns on, each named once: at least one, except for `normal`, which may be empty (`preset.empty_normal`) |

A switch needs `set`, `off` or both.

## Profile names

A profile is any set of switches. Tools take a profile by name: `retro` for none, the name of a preset or a switch, or names of switches and presets joined by `+`, such as `normal+slow_slugs`. The set's own name is its switches' names sorted and joined by `+`, such as `easy_stomp+slow_slugs`. That is what applying it writes into `model.profile` and what the state hash covers, so a preset and the same switches named one by one run the same.

## Rules

| Rule | Checked by |
|---|---|
| Each path is a concrete [model path](model-path.md): no `[*]`, range or filter | `gp model check` |
| Each path names a part the model has | `gp model check` |
| Each path is a mechanic's parameter or option: classed `parameter` or `mechanic` with its mechanic named ([mechanics](../mechanics.md)), under `entities`, `cameras`, `levels`, `counters` or, since `switch.rules`, `rules` and `input.keep_direction`. Content, presentation and the header are refused | `gp model check` |
| A path is named once in a switch | `gp model check` |
| No two switches name the same part, a part and a part inside it, or two items of one list, so any switches combine and the order they're applied in doesn't matter | `gp model check` |
| Every switch has a description | `gp model check` |
| A preset turns a switch on, unless it is `normal` | `gp model check` |
| A game's file has a `normal` preset, which turns on every switch of the group `timing` | The gp-model test `every_template_keeps_to_the_settings_standard`, over the tool repo's `templates/` (`Profiles::standard_problems`) |
| Group, switch and preset names follow the name rule; switch names are unique and not a preset's; a preset names only switches the file has | `gp model check` |
| The model each switch makes alone, each preset makes, and every switch at once makes parses and is valid. Evidence entries left covering nothing by `off` are allowed | `gp model check` |
| `model.toml` itself has no `model.profile` | `gp model check` |

## Applying one

Applying a profile copies the model, writes each `set` value and removes each `off` path of every switch on, then sets `model.profile` to the set's name (`Profiles::apply`). The copy is what a runtime runs; the model directory isn't changed, and the model's canonical text and hash stay the base's. The state hash in a profile other than `retro` covers the set's name ([the model's "Profiles"](../model.md#profiles)). `gp interp run --profile NAME` runs a model in one.

## Versions

| Version | Changes |
|---|---|
| 0.2.0 (2026-10-30) | Profiles become switches. `profiles.<name>` is gone: a file has `groups` of `switches`, each with a name, a description and its `set` and `off`, and `presets` naming sets of them. Switches may not change the same part. A profile's name is `retro`, a preset, a switch or names joined by `+`, and `model.profile` holds the switches on, sorted and joined by `+`, so the state hash covers the set. A 0.1.0 file is refused: turn each profile into a switch in a group, and add a preset of the same name if a tool asks for it by name |
| 0.1.0 (2026-10-30) | First version, with core 0.4.5 |

## Additions without a version bump

| Name | What it adds |
|---|---|
| `preset.empty_normal` | The preset `normal` may turn no switch on, for a model with nothing to turn off, as Moss Hop's ([the settings screen](../settings.md)). A reader without it refuses the empty list as it did before. Not in the feature registry, as `switch.rules` isn't |
| `switch.rules` | A switch may change a game rule's parts, such as `rules.game.states.dying.transition[0].after`: a rule's timing or flow is an engine behaviour a switch can turn off like a mechanic's option. A reader without it refuses such a path as it refused before. It isn't in the feature registry, which derives features from model files and not from this one |
