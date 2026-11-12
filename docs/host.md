# The local host

`gp host` and crates/gp-host came from game-printer whole in wave 7 of the port; "In gameprinter" below says what changed. The rest of this page is game-printer's, with paths edited; its links to pages gameprinter doesn't have (poc, capture live) mean game-printer's docs/.

game-printer's Milestone 3 agent experience needed more than tools. Something has to keep a game loaded while an agent changes it and run the checks that say what the change did. It also has to build in the background and hand back a link a person can click. `gp host` does this on one machine. This page also covers the consistency critic and the speed targets.

The host is local by the user's decision of 2026-10-28, with no network service and no accounts. The interface stays open. [`gp_host::Host`](../crates/gp-host/src/lib.rs) is a Rust trait whose methods take and return serde types, and `LocalHost` is its one implementation. A hosted runtime would implement the same trait over the network. The CLI, the agent loop and the tests would not change.

## Run it

```sh
gp host serve --repo . --work WORK --assets LIFTED   # sessions, jobs and links on http://127.0.0.1:7710/
gp host scenarios mosshop --template                 # run a template's scenario tests, in parallel
gp host scenarios COPY                               # or a working copy's
gp host record COPY NAME --frames 300 --hold ...     # record a run of the copy as a scenario of its own
gp host search COPY --baseline finish-level --goal rules.game.state=won --progress 'entities.player[0].x'
gp host speed --plan 2.1 --first-link 40 --tweak 0.4 # hold timings to their budgets
just host-mosshop                                    # Moss Hop's scenarios (in just check)
just poc-mosshop                                     # the whole loop, host included (docs/poc.md)
```

`--work` is the directory `gp mcp` uses for working copies (default `$GP_HOME/work/mcp`), so the host and the MCP server see the same copies. It must be outside any git work tree. The host keeps its own files in `WORK/.host/`: `links.json`, `builds/`, a shared cargo target directory for Bevy builds, and short-lived snapshots.

## In gameprinter

The crate came over unchanged, with its 14 tests. The command lost what fed game-printer's run recorder: `host scenarios` and `host speed` record no metrics, and their JSON has no `run_manifest`; otherwise it is the same. Scenarios and the input search live in gp-mcp, as before.

The `capture` job runs `gp capture start` and `gp capture watch`, the capture-run loop the editor port brings back; until it lands, a capture job (and `gp host serve --capture RUN`) fails with gp's error. The `build` job with `target = "web"` needs `gp gen bevy`, which gameprinter dropped; the default `web-preview` build works.

On 2026-11-12, old binary against new on Moss Hop: `host scenarios mosshop --template` gave the same results but for the timings (`millis`); `host speed`, `host search` (from stomp-then-pit, 20 rounds) and `host record` on a forked copy gave equal JSON; `host serve --port 0` printed its URL line and answered `GET /api/jobs` the same.

## The interface

| Method | What it does |
|---|---|
| `open_session`, `step_session`, `query_session`, `close_session`, `sessions` | Interpreter sessions on a working copy |
| `run_scenarios`, `record_scenario`, `search_inputs` | Scenario tests, run now |
| `submit`, `job`, `jobs` | Background jobs: a build, or a scenario run |
| `register_link`, `links` | Playable links |
| `speed`, `record_speed` | The speed metrics |

`LocalHost::start` runs jobs on worker threads of this process, and `gp_host::http::serve` puts a JSON API on `127.0.0.1` in front of any `Host`. The listener has no authentication, as `gp serve` has none, and so refuses every other address.

## Sessions

A session is a copy's model loaded in the interpreter and kept between calls, so an agent can step a game, change it and step again without paying the load. `step` runs frames from `frames` and either `buttons` (held for every frame of the step) or `holds` (spans relative to the step's first frame, such as `a@0-10`) and returns the frame, the state hash, the events and any `paths` asked for. `query` reads state without running. Each call first compares the copy's `model.toml` with what the session loaded. If it changed, the session reloads the new model and replays every frame it has run, so the state it reports is the edited game's on the same inputs, and `reloaded` says so. At most 32 sessions are open.

## Scenarios

A scenario is a short scripted run with expectations. A template carries them in `scenarios.toml`, and `template_fork` copies the file into each working copy, so an agent can ask after any change whether the game still does what it did. Version `0.1.0` of the format looks like this.

```toml
format = "0.1.0"

[scenarios.jump-height]
description = "A jump started at frame 50 peaks near frame 69, 47.5 pixels up, and lands"
frames = 130
holds = ["a@50-60"]          # as run_preview takes them; or baseline = "finish-level"

[[scenarios.jump-height.expect]]
at = 69                      # the frame the value is read at (default: the last)
path = "entities.player[0].y"
op = "between"               # eq ne lt le gt ge between
min = "124"
max = "133"

[[scenarios.jump-height.expect]]
event = "goal.reached"       # an event kind, counted over the run
count_min = 1

[[scenarios.jump-height.expect]]
state_hash = "00ab..."       # the interpreter's hash of the final state
```

Every expectation is exactly one of `path` with `op`, `event` with `count_min` or `count_max`, or `state_hash`. A `path` may be a pattern (`entities.slug[*].alive`), and then every match must satisfy it. Numbers compare as numbers (fixed-point values are exact decimal strings); flags and text compare for equality only. The file is checked when read: an unknown key, a scenario that expects nothing or a bad `op` is an error naming the scenario.

`run_scenarios` loads the model and assets once, then runs the scenarios on a pool of threads (`jobs`, default the cores up to 8), each with its own interpreter. The report lists every scenario with its checks and what each check got, and a `millis` block with the wall time and the sum of the runs, so the speedup is visible. The outcome without timings (the model's and the file's SHA-256, and each scenario's result) is written to the evidence store as one object, and its hash is `store_object` in the report. The same model and scenarios give the same hash for any `jobs` and on any machine, so a result can be quoted, compared and kept without keeping a log.

Moss Hop's six scenarios (`templates/mosshop/scenarios.toml`) run in about 5 ms on 6 threads and were a step of game-printer's `just check`. On a copy with the jump 20% higher, `jump-height`, `finish-level` and `slug-death` fail and the other three pass. [`templates/*/AGENTS.md`](../templates/mosshop/AGENTS.md) says which scenarios each recipe is expected to break.

### Variant baselines

The proof of concept ([poc.md](poc.md)) found a problem with a variant's validation. The original's recorded inputs stop winning once the jump is higher, so the outcome checks against the original fail, and loosening an Exact rule would pass them and say nothing. The answer is a baseline from the variant's own play.

1. `search_inputs` (`gp host search`) starts from a baseline's holds and moves each hold span (its start by up to `window` frames either way, its length by -2 to 4) for a few rounds, keeping any move that does better. Better means the goal is met at the last frame, then the furthest the `progress` path reaches. It is a local search, not a planner. On Moss Hop with the jump 20% higher it finds a winning run from `finish-level` in about 620 tries, and it says plainly when it can't.
2. `record_scenario` (`gp host record`) runs given holds on the copy's own model and appends a scenario to the copy's `scenarios.toml` that pins what happened: the final value of each counter and state machine, the event counts and the state hash.
3. The copy now has a run that wins by its own rules. A later edit that changes that run makes the scenario fail.

Validation against the original keeps running on what the edit doesn't touch (`validate` with `baselines` naming the ones that don't play through the changed jump). Together they are the default the open question asked for: the variant is checked against its model by its own recorded play, and against the original where the original still applies.

## Jobs and playable links

`submit` queues a job and returns at once. A job is one of three kinds.

| Kind | Does |
|---|---|
| `build` | Copies the working copy to `WORK/.host/snapshots/<job>/`, runs `build_publish` on the snapshot, registers a link for the output and deletes the snapshot |
| `scenarios` | Runs `run_scenarios`, as a job |
| `capture` | Watches a capture run ([watching capture live](capture-live.md)): `gp capture start` when `start` is given, then `gp capture watch`, which runs a round each time an answer or pin arrives and ends at a steady state. It runs on a thread of its own rather than a worker, since it mostly waits for a person. `gp host serve --capture RUN` submits one per run at start |

A build is of the copy as it was when the job started, so an agent can keep editing while it runs: the copy is locked only for the time it takes to copy it. The Bevy build's cargo target directory is shared by symlink across snapshots, since a cold build takes minutes. Workers are `--build-workers` (default 1; builds share one cargo lock). A job's view has `state` (`queued`, `running`, `done`, `failed`), `wait_ms`, `run_ms`, its result or its error, and for a build its link.

A link is a published directory served at `/play/<id>/`, where the ID is the first 12 hex digits of the SHA-256 of the build's `manifest.json`. The same build has the same link and a rebuilt one has a new link. The registry is `links.json`, so links survive a restart, and the host serves what is in the directory. Requests can't leave it. A path with `..`, an empty segment or a symlink out is a 404. `GET /` lists the links.

| Request | Does |
|---|---|
| `GET /api/health` | `{ok: true}` |
| `POST /api/sessions` `{copy}`, `GET /api/sessions` | Open, list |
| `POST /api/sessions/ID/step`, `/query`; `DELETE /api/sessions/ID` | Step, read, close |
| `POST /api/scenarios/run`, `/record`, `/search` | The scenario calls above |
| `POST /api/jobs` `{kind: "build", copy, target?, fetch_tools?}`, `{kind: "scenarios", copy, ...}` or `{kind: "capture", run, start?: {model, rom?, baselines?, recordings?, preview?, with?}, debounce_ms?, max_rounds?, idle_s?}`; `GET /api/jobs`, `GET /api/jobs/ID` | Queue and watch |
| `POST /api/links` `{dir}`, `GET /api/links` | Register a build, list links |
| `GET /api/speed`, `POST /api/speed` `{name, seconds}` | The speed metrics |
| `GET /play/ID/...` | A build's files |

Errors are `{error}` with 400 (the body isn't the right JSON), 404 or 422 (the call is well formed and the host refused it).

## Critic

The consistency critic checks each generated asset against the art already in the copy. `gp_mcp::critic::Critic` is a trait that takes a `Subject` (the candidate's image or palette, the palettes it would be drawn with, the level's backdrop color, the other sprites of the same sheet and the other tiles slots' sheets, and the person's prompt) and returns a `Review` with a score from 0 to 1, a `pass` flag and a list of checks. An image model that judges a picture would implement the trait, take the images as PNG (`Image::to_png`) and return the same `Review`. The one built in, `Metrics`, is deterministic and offline, with fixed thresholds and no randomness, so the same candidate in the same copy gets the same review.

| Check | Looks at | Fails when | Warns when |
|---|---|---|---|
| `shape.ink` | Share of a sprite's pixels that are drawn | Blank, or solid with no silhouette | Outside the role's range (for example 0.30 to 0.95 for a player) |
| `shape.bbox` | How much of the box the drawing spans | Never | Under 0.5 on either axis |
| `size.siblings` | Ink density against other sprites of the sheet (the same role if there is one) | Never | More than 0.35 from the median |
| `palette.distinct` | The sprite's palette | Two of colors 1 to 3 are equal, or one equals the backdrop | Never |
| `palette.hues` | Every palette of the game | Never | More than 8 distinct hues |
| `palette.ramp` | One palette's light-to-dark spread (relative luminance) | Never | Under 0.15 |
| `palette.harmony` | A palette's brightness against the others' mean | Never | More than 1.5 brightness steps |
| `contrast.backdrop` | The best contrast of a color the sprite draws with against the backdrop, as the WCAG ratio | Under 1.5 | Under 3.0 |
| `style.similarity` | Ink share, outline share and color mix against the other art | Never | Distance over 0.35 |

A check with nothing to compare against is `skipped` and does not count. The score is passes plus half of warnings, over the checks that ran. NES colors are turned into RGB by a fixed table of twelve hues shaded by brightness row (`critic::nes_rgb`), which is close enough for a contrast ratio and is not any emulator's palette.

`assets_generate` runs the critic on every candidate. `critic: "advise"` (the default) returns the review and places the candidate. `"enforce"` places it only if the review has no failure and scores at least `min_score` (default 0.7). `"off"` skips it.

## Progressive generation

`assets_generate` with `progressive: true` fills every presentation slot in the order the catalog implies, so each piece is made and reviewed against what is already there:

1. Palettes slots first. Every sprite's colors and the critic's contrast and harmony checks depend on them.
2. Each tiles slot that still holds an original gets a base sheet of stub tiles, so no tile of an original stays in it. Slots filled from one asset get one base step.
3. For each tiles slot, its sprites by role: player, enemies and bosses, characters, pickups, projectiles, hazards, then the rest, ties by entity name. The player is first because later sprites are compared with it.

Slots the simulation reads are skipped (`model_edit` changes those). For a sprite the tool tries up to `tries` seeds from `seed` (default 4, at most 16) without placing, reads each review, and places the first that passes with `min_score` (default 0.7), or else the best. `plan_only` returns the order. Each step in the reply has its seed, score and warnings, and says whether the baselines' state digests held. On Moss Hop the order is palettes, the base sheet, the player, the slug.

The generators are the three deterministic ones, so the art is placeholder quality. The critic and the order are what an image model would plug into: it becomes a fourth `Generator`, its candidates go through the same review and the same placement checks, and `tries` becomes a retry budget.

## Speed targets

The roadmap sets three targets, kept as metrics so a regression shows (`gp_host::speed`), and capture adds a fifth:

| Metric | Measured from | To | Budget | Target |
|---|---|---|---|---|
| `plan_s` | The task starts | A concept plan: the template found, forked, its surface read, the numbers to compare against taken | 30 s | 10 s |
| `first_link_s` | The fork starts | The first playable link answers (placeholder art, the original game) | 120 s | 60 s |
| `tweak_s` | A model edit is sent | Its effect read back from a preview run | 5 s | 1 s |
| `tweak_link_s` | A model edit is sent | The rebuilt link answers | 30 s | 10 s |
| `rom_to_preview_s` | `gp capture start` is given a ROM | The run's first interpreter preview is built ([watching capture live](capture-live.md)) | 60 s | 20 s |

The POC loop measures them with its own clock around the MCP calls and the host's job record, so they are tool time and not a model's thinking time. It sums the calls an agent has to wait for and leaves out the checks the script makes of its own. `just capture-mosshop` measures `rom_to_preview_s` on a fresh `$GP_HOME`. `gp host speed` holds them to the budgets and exits 1 when one is over. In game-printer it also recorded each as a run metric (`speed.plan_s` and so on) for its metrics database; gameprinter has no run recorder. The host also keeps a log of its own timings (`build_s`, `scenarios_s`) at `GET /api/speed`.

## Measured

Numbers from `just poc-mosshop` on 2026-10-28 (Apple M-series, debug `gp`, warm build):

| What | Time |
|---|---|
| Moss Hop's 6 scenarios on 6 threads (`gp host scenarios mosshop --template`, debug) | About 5 ms wall, 12 ms serial |
| 7 scenarios with the recorded variant baseline, 4 threads | 3 ms |
| The input search for the 20% higher jump (621 tries, debug) | About 9 s |
| `plan_s` | 0.08 to 0.11 s |
| `first_link_s` | 1.2 to 2.1 s |
| `tweak_s` | 0.04 to 0.10 s |
| `tweak_link_s`: a build job of a snapshot, queued to answering | 1.1 to 1.6 s |
| The whole loop, with the host and the GIF | 29 to 35 s, 39 tool calls |
| Preview module, unshrunk and with `wasm-opt` | 1.55 MB and 1.37 MB |

The build timings are warm: the cargo target directories already held `gp-web` for the browser, so a build is the model's preparation and a copy of the module. A first publish on a clean machine compiles `gp-web` for wasm32 and takes longer; it was not timed separately, and the budget of 120 s is the roadmap's "1 to 2 minutes". The same run with `--fetch-tools` took 85 s, of which about 46 s was the first publish fetching the tools.

## Limits

- Everything is on one machine. The host has no authentication, a single user and no quotas. Sessions and jobs are in memory; links are on disk.
- `gp host` and `gp mcp` are two processes sharing one work directory. The copy lock is in-process, so a build snapshot taken while an MCP call is mid-edit could see a half-done edit; the POC runs one after the other. A hosted runtime owns the copies and takes the lock itself (an open question).
- Builds share a cargo lock, so parallel builds mostly wait. A hosted runtime has a build farm and a content cache.
- The search finds inputs near the original's. It will not find a run through a level the edit made very different.
- The critic judges measurable properties. It can't say a sprite looks like a moss creature.
- Only Moss Hop ships scenarios. The commercial templates' expected numbers depend on the person's lift, so their `AGENTS.md` tells the agent to record scenarios from the unedited copy.
