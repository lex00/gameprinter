# Agent tools over MCP

`gp mcp` serves Game Printer's agent tools to an agent over MCP. It came from game-printer (crates/gp-mcp whole, and gp-cli's mcp.rs) in wave 7 of the port; "In gameprinter" below says what works here and what waits for other commands. The rest of this page is game-printer's, with paths edited; it was written for Milestone 3 there, and its links to pages gameprinter doesn't have (stubs, sprite editing, sound, show, point and tell, capture live) mean game-printer's docs/. It began as a research spike and was adopted by the user on 2026-10-28. The roadmap's agent tools are `registry.search`, `template.fork`, `surface.describe`, `model.edit`, `contract.edit`, `assets.generate`, `assets.place`, `run.preview`, `validate` and `build.publish`. This page describes the server and those tools, with three more: `model.refine`, `model.questions` and `show.explain`. The agent skill that went with it is game-printer's docs/agent-skill.md.

The code is `crates/gp-mcp`, with the command in `crates/gp-cli/src/mcp.rs`.

## Run it

```sh
just mosshop
gp assets lift target/mosshop/mosshop.nes -o ~/.gameprinter/work/assets/mosshop --map assets/mosshop.toml
gp mcp --rom target/mosshop/mosshop.nes
```

`gp mcp` speaks MCP on stdin and stdout until stdin closes. It takes no `--json`, since stdout is the MCP channel. An MCP client starts it as a local command; there's no network transport. Its options:

| Option | Default | What it is |
|---|---|---|
| `--repo DIR` | The git work tree of the current directory | Where `templates/` and `baselines/` are, and what evidence refs are read against |
| `--work DIR` | `$GP_HOME/work/mcp` | Working copies, and the store validation keeps results in (`DIR/.store`). It must be outside any git work tree |
| `--rom FILE` | Found in `$GP_ROM_DIR` by hash | The original's ROM, for validation |
| `--assets DIR` | `$GP_HOME/work/assets/<model.source>` | Lifted assets, for previews and validation, and copied into a working copy's own `assets/` by its first `assets_place` |

## In gameprinter

The crate came over whole, with these changes: its generator calls go to gp-look (gameprinter's home for game-printer's gp-gen-bevy `present` module), the catalogue no longer describes Super Mario Bros.' area and enemy tables (cut from gp-assets), two tests name Donkey Kong where they named that game, and the check no longer passes `--root` to `gp model check`, which gameprinter's doesn't take. Its 83 unit tests pass.

Many tools run `gp` as a child process. Those whose command gameprinter has work; the others return that command's error as the tool's error until it lands:

| Tool | Needs | In gameprinter |
|---|---|---|
| registry_search, template_fork, surface_describe, model_edit, contract_edit, run_preview, assets_*, sound_* | gp-mcp itself, `gp model check`, `gp roms scan` | Yes |
| watch_start, watch_events, watch_watch, watch_hook, watch_explore, watch_point | `gp watch` | Yes (wave 3a) |
| watch_detail | `gp watch detail`, over `gp reach` | Once wave 3b lands |
| validate | `gp validate` | Wave 5 |
| model_refine, model_questions, refine_moment | `gp model refine`, `gp model questions`, `gp input-log generate` | Not yet |
| moments | `gp interp moments` | Once wave 2 lands |
| build_publish | scripts/web/build-preview.sh; with `target = "web"`, `gp gen bevy` | The preview yes; the Bevy build no (the generator was dropped) |
| show_explain, requests, capture_events, capture_input, view_now | `gp serve`'s sessions, `gp request explain`, the capture-run loop | When the editor port brings `gp serve` back |

tests/mcp.rs (game-printer's, end to end over stdio) runs four tests here; seven are ignored with the command each needs named, and the one that searched for game-printer's other templates searches for Moss Hop only.

## How it's built

The server is Rust, in the workspace, on `rmcp` 3.8.0, the official Rust SDK (docs/open-questions.md, "Which MCP implementation do the agent tools use?"). It uses only the SDK's server and its stdio transport.

Each tool is a plain function from JSON arguments to a JSON reply (`gp_mcp::tools::call`), so tools test without MCP, and the MCP layer (`gp_mcp::server`) only lists them and wraps replies. Arguments are Rust types whose JSON Schema `schemars` generates, as it does the model's. So a tool's schema and its parser can't disagree.

Work is split by where the code already is:

| Work | Where it runs | Why |
|---|---|---|
| Reading a model and contract, applying edits, the interpreter | In the server, on `gp-model`, `gp-contract` and `gp-interp` | These are library crates, and the edits need the model tree |
| `gp model check` and `gp validate` | `gp --json` as a child process, the same binary | These live in `gp-cli`, a binary crate. A child process gives the CLI's answers and exit codes exactly, records the same runs and metrics, and a crash in a run can't take the server down |

The child's stdin is closed, since the server's stdin is the MCP channel.

## Registering it with an agent

Any MCP client can launch it as a command. In a config file of the form `claude --mcp-config` reads, name the server `gp`; its tools are then `mcp__gp__*`:

```json
{"mcpServers": {"gp": {"command": "/path/to/gp", "args": ["mcp", "--repo", "/path/to/game-printer"], "env": {"GP_HOME": "/home/me/.game-printer"}}}}
```

`gp serve` writes this file itself for the agent it starts on a point-and-tell request, and lets that agent call `mcp__gp` and nothing else.

## Working copies

A tool never writes to the repo. `template_fork` copies a template's files into `<work>/<copy>/`, and every other tool names that copy. A copy's name is a name in the model path grammar (letters, digits and `_`), so it can't reach outside the work directory. Evidence refs are read against `--repo`, so a copy outside the repo still passes `gp model check`.

## Names

MCP allows dots in tool names, but some clients allow only letters, digits, `_` and `-`, so each tool is the roadmap's name with `_` for `.`: `surface.describe` is `surface_describe`.

## Results and errors

Every reply is a JSON object, sent as the result's `structuredContent` and also as its text, for clients that read only text.

| What happened | Reply |
|---|---|
| The tool did what was asked | `isError` false |
| A check ran and failed (`gp validate` exit 1, `gp model check` exit 1 inside `validate`) | `isError` false, with `ok` false. A failed check is a result |
| An edit was refused: it doesn't parse, or `gp model check` fails on it (exit 1) | `isError` true, with `ok` false, `written` false and the `problems`. Nothing is written |
| Arguments that don't fit the schema, a missing working copy, or a `gp` run that exits 2 | `isError` true, with `error`, the message |
| A tool that doesn't exist | A JSON-RPC error, -32602 |

Each tool carries the MCP annotations. `readOnlyHint` marks the tools that leave the working copies as they are (`validate` still keeps its results in the store), and `destructiveHint` the ones that may replace a working copy's files. Nothing reaches outside the machine (`openWorldHint` false).

## The tools

| Roadmap name | MCP name | Status | Backed by | Read-only |
|---|---|---|---|---|
| `registry.search` | `registry_search` | Built, the repo's `templates/` and `sources/` | `sources/*.toml`, each `template.toml` and model, `gp roms scan`, and the mechanic index (`gp_mcp::registry::mechanics`, [originals](originals.md#the-mechanic-index)) | Yes |
| `template.fork` | `template_fork` | Built, local templates only | Copies `templates/<name>/` | No |
| `surface.describe` | `surface_describe` | Built, the whole model | `gp-model` (facts, evidence), `gp-contract` (levels) | Yes |
| `model.edit` | `model_edit` | Built | `gp_model::path`, `Model::to_toml`, `gp model check` | No |
| `contract.edit` | `contract_edit` | Built | `gp_contract`, `gp model check` | No |
| `assets.generate` | `assets_generate` | Built: deterministic generators, a consistency critic on every candidate, progressive generation across slots. No image model | `gp_mcp::generate`, `gp_mcp::critic`, `gp_mcp::progressive`, then `assets_place`'s checks | No |
| `assets.place` | `assets_place` | Built, tiles and palettes slots ([stubs](stubs.md)) | `gp-assets` (pack), `gp-interp` (state digests), `gp model check` | No |
| `assets.edit` | `assets_edit` | Built ([sprite editing](sprite-editing.md)): deterministic semantic ops on the frames of an entity's sprite set or on its palette, or on a level tile kind's picture or background palette (`tile`), gated on the model knowing the sprite, drawn into the recorded frames with readability against their backgrounds, proposals, accept and reject, undo, redo, resize | `gp_mcp::sprite_ops`, `gp_mcp::sprite_set`, `gp_mcp::ready`, `gp_mcp::contexts`, `gp_mcp::scene`, `gp_mcp::edit_log`, `gp_mcp::resize`, `gp_mcp::tile_edit`, the critic, then `assets_place`'s checks | No |
| `assets.history` | `assets_history` | Built: a copy's art edits with their status and who decided | `gp_mcp::edit_log` | No |
| `assets.export` | `assets_export` | Research prototype: one sprite as an indexed PNG, a `.gpl` palette and a JSON sidecar for an external editor; `assets_edit`'s `load` op brings the saved file back | `gp_mcp::exchange` | No |
| `sound.banks` | `sound_banks` | Built: the APU synthesizer's voices and every SF2 and SFZ in the bank folders, each patch with a one-second preview WAV ([sound](sound.md)) | `gp_sound::banks`, `gp-sound-bank` | Yes |
| `sound.describe` | `sound_describe` | Built: a copy's songs and effects, their instruments, roles and targets, origin, confidence, bindings and readiness | `gp_sound::song`, `gp_sound::edit::readiness` | Yes |
| `sound.edit` | `sound_edit` | Built: deterministic ops on songs and the sound bank mapping, gated on readiness, with before and after clips, history, undo, a theme's restyled-audio layer, and a post to a live capture run | `gp_sound::edit`, `gp-sound-bank` | No |
| `sound.preview` | `sound_preview` | Built: the copy in the interpreter on a baseline's input, its music and effects cued as the game cues them, into a WAV | `gp-interp`, `gp_present::sound`, `gp_sound::player` | Yes |
| `run.preview` | `run_preview` | Built, no picture | `gp-interp` | Yes |
| `validate` | `validate` | Built | `gp model check`, `gp validate --runtime interp` | Yes, besides its result cache |
| `show.explain` | `show_explain` | Built: a play session's marks with what the original did there that the model didn't ([show](show.md)) | The session's `play.json` under `$GP_HOME/capture/`, written by `gp serve` from `gp show explain` | Yes |
| `build.publish` | `build_publish` | Built, local only: the interpreter preview, optionally the Bevy game | `scripts/web/` ([web.md](web.md)), `gp gen bevy`. A playable link on localhost comes from `gp host` ([host.md](host.md)); a link others open and the publish check are Milestone 5 | No |
| `model.refine` | `model_refine` | Built: records the source's baselines, then `gp model refine` on the copy ([mechanics](mechanics.md)) | `gp record`, `gp model refine` | No |
| `model.questions` | `model_questions` | Built: lists the copy's questions and writes chosen answers | `gp model questions` | No |
| | `capture_events` | Built: a capture run's events after a sequence number, the questions still open and the inputs waiting ([watching capture live](capture-live.md)) | `gp-capture` | Yes |
| | `requests` | Built: the person's point-and-tell requests in a run with every reply; `explain` runs `gp request explain`, `reply` answers one with plain text and a proposal item or `needs_mechanic` ([pause, point and tell](point-and-tell.md)). `gp serve` starts `claude -p` with only these tools on each explained request ([who acts on a request](point-and-tell.md#who-acts-on-a-request)) | `gp_capture::requests`, `gp request explain` | No |
| | `moments` | Built: the game's own moments in a capture run's recording with what the model does at each, and the one the person's words name (`said`, with `frame` for "this one") in `found` ([moments in words](point-and-tell.md#moments-in-words)) | `gp interp moments`, `gp_interp::moments::find` | Yes |
| | `refine_moment` | Built: asks for the refinement loop narrowed to one moment (its frames, its mechanic's numbers, the check on the others of its kind); the workspace starts it when the agent replies | the run's `refine/ask.json`, `gp model refine --lockstep --moment` | No |
| | `capture_input` | Built: an answer, pin, unpin or note sent to a capture run as the agent | `gp-capture` | No |
| | `watch_start` | Built: a watch session on a ROM, run in the background ([watching stage 0](watch.md)) | `gp watch start` | No |
| | `watch_events` | Built: a session's events after a `seq`, by kind, waiting up to 30 s for new ones while it is written | `gp-capture`'s reader on the session's file | Yes |
| | `watch_watch`, `watch_hook` | Built: the session's recipes run again with addresses watched or instructions hooked through the cartridge shim, the hits as `watch` and `hook` events | `gp watch rerun` | No |
| | `watch_detail` | Built: L3 for a window of a recipe, as `gp reach --detail` gives it | `gp watch detail` | Yes |
| | `watch_explore` | Built: another search in the session, in the background | `gp watch explore` | No |
| | `watch_point` | Built: the sprites under a box at a frame and the routine that wrote each | `gp watch point` | Yes, besides the point it logs |
| | `view_now` | Built: what the person's watch page shows and the last point | The session's `inputs.ndjson` | Yes |

Every tool also takes an optional `run`, a capture run's name. A call that names one is appended to the run's inputs as the agent's `tool` input (the tool, its arguments and its result, each cut to 200 characters), so the person watching the run sees the agent's calls in the run's log beside the pipeline's stages; `run` is taken out before the tool reads its arguments, and a run that isn't there is said in the reply as `capture_log` without failing the call. A call that carries a `progressToken` in its `_meta` gets `notifications/progress` every second while it runs (`progress` 1, 2, 3, ...), with the capture run's newest stage as the message when the call names a run, else how long it has run.

### template_fork

| | |
|---|---|
| Input | `template`, a directory under `templates/`; `copy`, the new working copy's name |
| Output | `copy`, `template`, `dir`, and `files`, the files copied |
| Errors | No such template; the copy exists; a name that isn't a path name |

A template's manifest, `template.toml` ([templates.md](templates.md)), declares its source by hash and its extraction recipe, and is copied with the rest. What else a template holds (scenario tests, change recipes) is still open in Milestone 3. A commercial-game template ships with empty slots (AGENTS.md), so forking one should run `gp template extract` on the user's ROM from `$GP_ROM_DIR` into the copy's own assets; today `template_fork` copies the files and validation reads the assets `gp assets lift` wrote (open-questions row, M3).

### surface_describe

| | |
|---|---|
| Input | `copy` |
| Output | `model`: the header (name, title, source, schema, extensions, view, systems, start level). `input`: the device and actions. `entities`, `levels`, `rules`, `counters`: their names. `slots`: the slot catalog ([stubs](stubs.md)), each slot's kind, role, `from`, item roles, shape, the entities that draw from it, whether the simulation reads it, its contract level, status and provenance, and the checks a replacement must pass. `contract`: the default level, each rule with its `index`, and `facts_by_level`. `facts`: every fact by concrete model path, with its `value`, the `level` that governs it, the `tolerance` of a metric level, the `rule` that sets it, and its `evidence` (key, source, confidence) |

On Moss Hop that's 95 facts (about 17KB of JSON), and the tool list is about 10KB. Today it describes the whole model. A template's change recipes come with it as `recipes` (the copy's `AGENTS.md`, up to 64 KB) and its scenario tests as `scenarios` (name and description of each entry in `scenarios.toml`). The modification surface proper is still to do. It will lead with the parts a template invites changes to, and the tool will keep the full list behind an option.

### model_edit

| | |
|---|---|
| Input | `copy`; `edits`, each a concrete model `path` with a `value` or `remove: true`; optionally `evidence` (`source`, `confidence`, `note`, `refs`) to record for every path set |
| Output | `ok`, `written`, `changes`: each change by model path with `old` and `new`, from `gp_model::diff`, and `evidence_dropped`: the evidence entries a removal left naming nothing, which go with it |
| Refused | `ok` false, `written` false, `problems` as `gp model check` gives them |

An edit names one part. A path may name an existing part, a new key in a table that exists, or the position one past the end of a list, which appends. A pattern (`[*]`, a range or a filter) is refused. So is a path into the evidence table or the `model` header. The header names the model and where it comes from, which an edit shouldn't change. Values are JSON, and a table or list is written as JSON too. A fraction is an exact decimal string, as in the model (`"0.1875"`); a JSON float is refused.

All the edits in one call apply to the model's tree in order, then the tree is parsed as a model and written in canonical form (`Model::to_toml`), so the file is always what `gp model fmt` writes and an edit to one fact changes one line. Then `gp model check` runs on the copy, with `--root` the repo and `--assets` the lifted assets when they're there. If it fails, the old file goes back.

An edited fact keeps the evidence that covered it unless the call gives `evidence`. That's wrong for an agent's own choice, which should be `inferred`; the open question is whether `model_edit` should require it (see "What's left").

### contract_edit

| | |
|---|---|
| Input | `copy`; `ops`, each one of `{"op": "add", "rule": R}`, `{"op": "set", "at": A, "rule": R}`, `{"op": "remove", "at": A}`, `{"op": "default", "level": L}`. A rule is a contract rule as in [the contract format](formats/contract.md). `at` is a rule's position, or a path that exactly one rule has |
| Output | `ok`, `written`, `rules` (each with its `index`), `default`, and `levels_changed`: each fact whose level changed, with `old` and `new` |
| Refused | As `model_edit` |

The operations apply in order, so a position after a `remove` counts the rules that are left. The contract is written in canonical form and checked with `gp model check`, which checks it against the model, `when` conditions included.

### run_preview

| | |
|---|---|
| Input | `copy`; the input as `frames` (default 60) and `holds` in the baseline syntax (`a@60-70`, `right+b@10-`), or `baseline`, a baseline of the model's source by name; `paths`, state paths to read (default: every value); `at`, frames to read them at besides the last; `picture` |
| Output | `frames`, `state_hash` (the protocol's 8 bytes, in hex), `events` (`frame`, `kind`, `entity`; at most 1,000, and `events_dropped` past that), and `samples`, each a `frame` and its `state` by path |
| Errors | No lifted assets; a path the interpreter doesn't answer; more than 36,000 frames; more than 1,000 frames in `at`, or one past the end |

State paths are the interpreter's ([the interpreter](interpreter.md#state-by-model-path)): `entities.player[0].y`, `entities.slug[*].x`, `rules.game.state`, `counters.deaths`. Integers and flags come back as JSON numbers and booleans, fixed-point values as exact decimal strings. A baseline's input plays as `gp interp run --baseline` plays it, frame for frame, without the original's lag frames taken out.

There's no picture. The runtime protocol's `present` interface isn't implemented by any runtime, and `gp video` renders only ROM recordings. The playable web build's preview ([web.md](web.md)) draws a model's state in the browser from `gp_web::Preview::draw_state`, but nothing renders that to an image the server could return. With `picture: true` the reply says so. A picture comes with `present`, which the interactive workspace (Milestone 2, step 6) needs for its variant previews; until then a link to the web preview of the copy is the nearest thing (open-questions row, M3).

### validate

| | |
|---|---|
| Input | `copy`; `baselines`, names of baselines of the model's source (default: all of `baselines/<source>/`); `scenarios`, to also run the copy's scenario tests; `scenario_names`, to run only some |
| Output | `ok`, `stage` (`model check` or `validate`), and for `validate`: `passed`, `failed`, `unexercised`, `failing` (the failed checks), `checks` (every check as `gp validate --json` gives it, with an `example` of what was measured and expected), `unchecked_facts` (a count per level), `recordings`, `subject`, `original`. For `model check`: `problems`. With `scenarios`, `scenarios` holds the run ([host.md](host.md#scenarios)): `ok`, `scenarios`, `passed`, `failed`, `store_object` (the hash of the stored outcome) and each scenario's result, with only its failing expectations |

It runs `gp model check`, then `gp validate COPY --runtime interp --against ...`. Results are kept in the server's store, keyed by the model's text among other things, so validating an unchanged copy again is quick.

A check against the model compares the original's run with the model's own numbers, and names the fact that's off. A check against the original compares the two runs, and one change can fail several. With Moss Hop's gravity lowered from 0.25 to 0.1875 the jump fails both ways. The higher jump also kills the player in `finish-level`. From there the game's state and counters part from the original's, as does the level graph.

### registry_search

| | |
|---|---|
| Input | All optional: `query`, words that must each start a word of a template's or source's id, title, description, entity names, roles or mechanics; `platform` (`nes`); `genre`, a model extension name (`platformer`); `roles`, entity roles every result must have (`boss`); `systems` (`platformer.enemies`); `mechanics`, mechanics every result must use, as `role:mechanic` (`enemy:contact.kick`) or a mechanic for any role (`jump.coyote`); `inspired_by`, a template the results took mechanics from; `like`, a template to rank by likeness to; `limit` (default 20) |
| Output | `templates`: for each, `template` (the directory to give `template_fork`), `id`, `title`, `tier`, `platform`, `genre`, `systems`, `roles` (a count per role), `entities`, `levels`, `slots`, `source` (its registry entry, how it's made and the ROM hashes it accepts), `mechanics` (its mechanic index), `inspired_by` (its manifest's provenance: `template`, `took`, `into`, `note`), `taken_by` (the templates whose provenance names it), `similarity` (with `like`: the share of mechanics in common with the example, 0 to 1), `needs` and `ready`. `sources`: registered sources by words and platform, with `rom_present` and the templates that use them. `templates_found` and `sources_found` count before `limit` |

`needs.rom` says whether an accepted ROM is in `$GP_ROM_DIR` (found by `gp roms scan`) or is the one the server was given. `needs.extracted` reports whether lifted assets are at hand: the server's `--assets`, `$GP_HOME/work/assets/<source>`, or where `gp template extract` puts them, and whether their digest is the one the template pins. `ready` is true when assets are there and not contradicted by the pin; `next` says what to do when they aren't. Sources have no model, so a `genre`, `roles` or `systems` filter leaves them out. With `like`, results come most alike first; otherwise a template whose directory or ID the `query` names comes before one that only mentions it. A `mechanics`, `inspired_by` or `like` filter leaves sources out too. An original template ([originals](originals.md)) is `ready` once `gp template build` has written its assets, and `next` says to run it until then. It reads the repo's templates and `sources/nes.toml`.

### assets_generate

| | |
|---|---|
| Input | `copy`; `slot`, a tiles or palettes slot; optionally `entity` (only its sprite's tiles, or its palette); `generator` (`stub`, `shapes` or `recolor`; default `shapes` for tiles, `stub` for palettes); `seed` (default 0); `hue_shift` (for `recolor`); `prompt` and `author` (recorded in the slot's note); `place` (default true); `critic` (`off`, `advise` by default, or `enforce`); `min_score` (default 0.7, for `enforce` and `progressive`); `progressive`, `plan_only` and `tries` (below) |
| Output | `generator` (name, seed, label), `candidate` (a base64 PNG and its `image_size`, or the palettes), `critic` (the review: `critic`, `score`, `pass` and `checks`, each with `name`, `verdict`, `value`, `limit` and `message`; null when `critic` is `off`), and, when placed, `assets_place`'s output. With `enforce`, a review that fails or scores under `min_score` gives `ok` false, `written` false and `stage` `critic` |
| Refused | As `assets_place`: the candidate is held to the slot's shape and the same checks run. A generator that doesn't fit the slot, a `hue_shift` that turns every color back to itself, or an unknown generator is an `error` |

The tool is deterministic and offline, so the same arguments give the same bytes. `stub` draws what `gp assets stub` draws, `shapes` draws a shape per entity role at the sprite's size (a whole sheet is stub tiles with every sprite drawn in), and `recolor` permutes a tiles slot's color indices or turns a palette's hues round the NES color wheel. The first two hold no one's work and are `CC0-1.0`; `recolor` keeps the license of the content it was made from. A whole-slot `stub` records `status = "stub"` and the others `replaced`, with `origin = "generated"`. A generator is a Rust trait (`gp_mcp::generate::Generator`) taking the checked slot and returning pixels or palettes of the right shape, so an image model can be added behind it without touching the checks. `place: false` returns the candidate and writes nothing.

The critic is `gp_mcp::critic::Critic`, a trait taking the candidate in the context of the copy (the palettes, the other sprites of the sheet, the level's backdrop) and returning a review. The one built in is deterministic: shape and size checks, palette coherence, contrast against the backdrop and style against the other art, each with a fixed threshold ([host.md](host.md#critic) lists them). An image model would be a second implementation behind the same trait.

With `progressive: true` the tool fills every presentation slot in an order the catalog implies: palettes first, followed by each tiles slot's base sheet and its sprites by role (player, enemies and bosses, characters, pickups, projectiles, hazards). For each sprite it tries up to `tries` seeds (default 4, at most 16) without placing and reads each review. It places the first to pass with `min_score`, or else the best. `plan_only` returns the order. The reply gives the seed and score of each step, with its problems and whether the simulation was unchanged.

### assets_place

| | |
|---|---|
| Input | `copy`; `slot`, a tiles or palettes slot; optionally `entity`, to replace only what its sprite uses (its tiles, or its one palette); for a tiles slot the image as `file` or `png_base64`, an indexed PNG of the slot's or the sprite's `image_size`; for a palettes slot `palettes`, lists of four NES color numbers from 0 to 63; `license` (SPDX); `origin` (`generated` or `uploaded`); optionally `generator`, `author`, `note` |
| Output | `ok`, `written`, `slot`, `entity`, `assets` (the copy's assets directory), `file`, `tiles_changed` or `palettes_changed` (and `shared_with`, the other entities that draw with a replaced palette), `bytes_changed`, `state` (the slot's new entry in `slots.toml`), and `simulation` with `unchanged`, `baselines` (how many ran) and each baseline's state digest |
| Refused | `ok` false, `written` false, `problems`: a wrong size, color index, palette count or color number, pixels past the last tile, a pack that fails, a changed digest, a failed model check; the files and `slots.toml` go back. A simulation slot (or one sharing its asset), a kind other than tiles and palettes, a source with no baselines or a license that isn't an SPDX expression is an `error` |

[stubs.md](stubs.md) gives each step. The copy gets its own `assets/` on first use, and `run_preview`, `validate` and model checks use it from then on. Replacing part of a slot (`entity`) records both licenses (`LicenseRef-own-copy AND CC0-1.0`). Slots filled from the same asset share its file, so a placement updates the state of all of them. `surface_describe` compares each slot's files with the digest `slots.toml` recorded and reports `status` `modified` (with `files_sha256`) when they differ, as after an edit by hand.

### assets_edit

[Sprite editing](sprite-editing.md) is the product doc. Level tiles go through the same tool with `tile` in place of `entity`, not a sibling tool: they share the ops and palette ops, the copy's history and decisions, and the previews and `assets_place`'s checks. A proposal's item is the same `<copy>:<seq>` the workspace already accepts and rejects ([sprite editing](sprite-editing.md#level-tiles)).

| | |
|---|---|
| Input | `copy`; `entity`, or `tile` for a level tile kind (its asset map name, `<slot>[<index>]` or its tiles in hex); optionally `slot` (default `tiles`, or the first palettes slot for `palette_ops`); `ops`, run in order on each frame: `recolor` (`pairs`, optional `region`), `flip`, `mirror`, `shift`, `outline`, `fill`, `pixels`, `copy`, `load`, `match_style` (`entity`), `tween` (`from`, `to`: frame ids); `frames` (frame ids, a clip's name, or `all`; default the sprite's own); `pictures` (whole frames by id, rows of indices); `import` (a folder an editor saved into, from `assets_export`); `editor`; `resize` (`[width, height]` in tiles); or `palette_ops` (`color`, `swap`, `hue`, `brightness`, `copy_from`) with `same_role`; `place` (default true; false makes a proposal); `in_context` (default true); `critic` and `min_score` as `assets_generate`; `prompt`, `author`; `run`, a live capture run to tell. Or alone: `undo` (with `seq` for an older edit), `redo`, `accept: SEQ`, `reject: SEQ` |
| Output | `readiness`, `contexts` (poses and moments in the baselines), `sprite_set` (its frames and those skipped), `frames` (each edited frame's ops, changed pixels, body checks), `ops`, `pixels_changed`, `ink`, `body`, `checks` (body and `context.readability`), `tiles`, `critic`, `preview` (close-up, base64), `in_context` (`shots`, `readability`, the strip as base64), `after`, `item` (`<copy>:<seq>`), `seq`, `status`, `previews` (files), and when placed `assets_place`'s output and `history`; a palette edit gives `palette`, `palettes_changed`, `shared_with`, `colors`; a resize gives `resize` (`tiles`, `free_tiles`); a tile edit gives `tile` (the kind's facts: slot, index, role, tiles, tiles slot, palette with its role and same-role palettes, where levels draw it, what shares its tiles and colors), `also_changes` (other tile kinds) and `also_changes_sprites`, and `contexts` as `seen` and `baselines` |
| Refused | `stage` `model` with the readiness list when the model doesn't know the sprite well enough; an op whose arguments don't fit, named by its frame and number; a resize without enough free tiles; then as `assets_place`. Undo, redo and accept refuse when the tiles changed since the edit |

### assets_history

| | |
|---|---|
| Input | `copy` |
| Output | `placed`, `undone`, `proposals` (each `seq`, `status`, `entity`, `slot`, `frames`, `ops`, `tiles`, `palettes`, `prompt`, `decided_by`), `waiting` (open proposals) and `next_seq` |

### assets_export

| | |
|---|---|
| Input | `copy`, `entity`, optionally `slot` |
| Input | also `editor`: `pixelorama` (default), `aseprite` or `libresprite`, or `png` |
| Output | `set` (the whole sprite set by the image format config: `dir`, `files`, `frames`, `config`, and `import`, the `assets_edit` call that brings it back); for a sprite whose own frame is a grid of tiles also `image`, `palette_file`, `sidecar`, `image_size`; `readiness`, `open_with` |

### sound_banks, sound_describe, sound_edit, sound_preview

Music editing is talking to an agent: there's no editor of its own in Game Printer, and FamiStudio and MIDI (`gp sound export` and `import`) stay the manual path ([sound](sound.md), "Editing by asking an agent").

| Tool | Input | Output |
|---|---|---|
| `sound_banks` | Optionally `copy` (its `sound/` folder is searched too) and `previews` (default true) | `folders`, and `banks`: each `name`, `kind` (`apu`, `sf2`, `sfz`), `path`, `license` and `patches` (`name`, `bank`, `program`, `keys` for an SFZ region, `preview` for the first eight) |
| `sound_describe` | `copy`, optionally `slot` and `song` | Each sound slot's songs: `kind`, `origin`, `driver`, `confidence`, `frames`, `loop_frame`, `tempo`, `channels`, `instruments` (`name`, `voice`, `role`, `notes`, `plays_through`), `bound_in_model`, `editable` and `missing`; and the mapping's `mix` and `banks` |
| `sound_edit` | `copy`, `ops` (`remap`, `role`, `transpose`, `tempo`, `mute`, `solo`, `gain`, `swap`; `gp_sound::edit::Op`), optionally `slot`, `place` (default true), `clip_song`, `seconds` (default 8, at most 60), `theme`, `run`, `prompt`; or `undo` | `said` (what each op did), `before` and `after` (WAV clips in `<copy>/previews/`), `history` (`seq`, `depth`), `live` (the run's inputs file) |
| `sound_preview` | `copy`, a `baseline`, or `frames` (default 600) and `holds` | `file` (a WAV in `<copy>/previews/`), `frames`, `seconds`, `cued` (each music change and effect with its frame), `notes` |

`sound_edit` refuses an op that names a song the model doesn't know completely, with `stage` `readiness` and the `missing` list. A song is known completely when capture is 80% sure of it, every instrument has a role and the loop is known, and when it's bound to a level or state (an effect to an event). Ops that change only the mix or remap by role in every song don't need a song to be ready. Every result is checked against the NES's channels; nothing is clamped. An edit writes the copy's `sound/<slot>.songs.json` and `sound/banks.toml` (or `themes/<id>/audio-banks.toml` and the theme's `audio` entry), appends the files as they were to `sound-edits.jsonl`, and `undo` puts them back. With `run`, the edit is posted to the capture run as a `sound_edit` input carrying `item` `sound:<number>` and, in `data`, its number and clips; the person answers with `gp capture input <run> --kind accept|reject --edit N` (or the workspace), and the agent undoes a rejected edit.

### build_publish

| | |
|---|---|
| Input | `copy`; `target`: `web-preview` (default, the interpreter preview) or `web` (the preview and the generated Bevy game, built for WebGL2 with the size-optimized profile, minutes when cold); `out`, a directory outside any git work tree (default `<work>/.publish/<copy>/<target>`); `dry_run`; `fetch_tools` |
| Output | `ok`, `written`, `out`, `index`, `manifest`, `serve` (a command that serves `out` on localhost). With `dry_run`, the same checks and the manifest without a build |
| Refused | `ok` false, `written` false, `stage` (`model check` or `slots`) and `problems`: the model check fails; a slot's files don't match the digest in `slots.toml`; a slot has no recorded provenance; any slot still holds an original asset of a commercial source |

There is no remote and no hosted runtime, so publishing is local: the directory is what a host would serve, and `index.html` plays it. The build runs `scripts/web/build-preview.sh` (and `build-bevy.sh` for `web`) as child processes, as `just web-mosshop` does. `manifest.json` (format 0.2.0) holds the template (`id`, `title`, `tier`) and the model's name, SHA-256 and canonical SHA-256 and the contract's. It holds the asset map's digest too, and every slot's `status`, `origin`, `license`, `generator`, `sha256` and whether the simulation or presentation reads it. The rest is the set of asset licenses and the baselines the preview replays, with the SHA-256 of every file in `out`. Format 0.2.0 adds three fields. `parent` names the template the copy came from (`template`, `found`, the SHA-256 of the template's model and contract, and the `content_sha256` that `gp template pack` printed, when the copy's `template.toml` records it in a `[parent]` table). `changes` lists what differs from that template. `model` has the model facts added, removed or changed, as `gp model diff` shows them, evidence aside. `contract_rules` lists the rules added, removed or whose level changed. `contract_set_free` is the paths now at `free` that weren't, and `slots` is every slot that is no longer original. `wasm` gives the preview module's size and whether `wasm-opt` shrank it. A viewer can tell a published Moss Hop with a Free jump from the original. Building again over an earlier build is allowed; a directory that holds other files is refused.

`build_publish` shrinks the preview module with `wasm-opt` when `$WEB_TOOLS` names the web tools or an earlier fetch left them in `$GP_HOME/work/web-tools/`. With `fetch_tools: true` it fetches them first (binaryen pinned by SHA-256 on macOS arm64; wasm-bindgen-cli built with `cargo install --locked`, which takes minutes the first time). When it can't, it ships the unshrunk module (1.4 MB against 1.2 MB) and says so in `wasm.note`. The `web` target always fetches.

The commercial-original rule (docs/stubs.md) refuses a slot when its license names `LicenseRef-own-copy`, which includes a slot only partly replaced, or when it is an original and the copy isn't from an open-tier template (a `template.toml` with `tier = "open"` in the copy). Presentation slots are replaced with `assets_generate` or `assets_place`. A simulation slot (a level, its tileset, spawn tables) can't be, so a copy of a commercial template needs new levels through `model_edit` before it can be published. This check refuses sharing by accident; it doesn't stop a person who edits `slots.toml` or `template.toml` by hand, and the publish check for others is Milestone 5.

## Measurements

On 2026-10-28, on this machine (Apple M-series, debug build):

| What | Time |
|---|---|
| `crates/gp-cli/tests/mcp.rs`, the whole stdio session below, with the ROM build and asset lift | 6.6 s |
| `validate` on all five Moss Hop baselines | About 2.2 s |
| `gp model check`, as each edit runs it | About 0.05 s |
| Building `gp-mcp` and its new dependencies from clean | About 10 s, 25 new crates (tokio, futures, chrono and their own) |

The tests are part of `cargo test --workspace`, so `just check` runs them.

- The first forks Moss Hop and describes it. A contract rule with a bad `when` is refused, and so is a float. Lowering the jump's gravity changes one line, and a preview shows the jump's peak 64.1875 pixels up. Validation then fails the jump against the model's numbers, with `ok` false. With the gravity put back the copy is the template byte for byte, and every check passes. It also checks that an unknown tool is a JSON-RPC error and that closing stdin ends the server with exit 0 and nothing more on stdout.
- The art tests replace the player's sprite, and find Moss Hop with `registry_search`, generate art into its sprite and palette slots and see all five baselines unchanged, and check a publish without building it.
- Unit tests in `crates/gp-mcp` run the placement, generation and publishing paths and the registry on a made-up game with a stand-in for `gp`. They cover a whole-slot placement and pixels past the last tile, and a refusal after a write that restores both files. They also cover two slots on one asset and calls on one copy taking turns.
- Two ignored tests build the preview and the Bevy game for real. `just check-cycle` runs them as the step `publish-mosshop`.

## What's left

| Item | Default |
|---|---|
| Evidence for agent edits | Require `evidence` on `model_edit` for every fact set, defaulting to `inferred` with the request as the note, so `gp model check` and `surface_describe` show what an agent chose |
| A copy's history | Keep each edit's model-path diff in the copy (a log next to `model.toml`), so an agent can undo by step and the editor can show what changed |
| Long validations | Progress notifications are sent (above). Still open: tasks (SEP-2663, which `rmcp` 3.8.0 supports) and cancelling a long call, before a long validation runs through the server |
| Writes that can't be seen half done | Done: a fork is copied under another name and renamed into place, and a call holds a lock on its working copy for its whole run, so two calls on one copy take turns. Still open: `model_edit` and `contract_edit` write the file and put it back on failure, so a server killed mid-edit leaves an unchecked file, and a preview in another process can read it. Check in a temporary copy and rename into place (open-questions row, M3) |
| The protocol | `rmcp` 3.8.0 knows revisions up to 2026-07-28, which replaces the `initialize` handshake with per-request metadata. The test client negotiates 2025-11-25. Follow the SDK, and pin it exactly |
| Previews from a snapshot | Fork the interpreter at a frame and preview variants from there, which `gp_runtime::Runtime::fork` already allows |
