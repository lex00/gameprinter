# The loop

Issue #43. The owner, 2026-11-19: "i want the loop to run agents autonomously with no agent controlling the loop at the highest level", and "this has to support parallel work for a real dev loop". The loop is a rules engine: `next` (crates/gp-cli/src/next.rs) reads only files and emits steps, and the steps, not an agent, decide what is done next. Before this an agent read `next` and improvised, and in the owner's experience an agent that controls the loop bends it.

This page is the first phase: step ids and the set of open steps, gp_run held to them, WRONG, which halts the loop, and the runner, which starts workers on the open steps in parallel.

## Steps and their ids

Every step `next` emits (`gp progress --next --json`, the MCP tool `next`, the page's status line) carries:

| Field | What |
|---|---|
| `id` | `s` and 12 hex digits: a hash of the step's subject, who does it, its command (`how.cli`, or its words when it has none), what it touches and the answer's `facts`, less the background's live counts (they differ between the page's server and a terminal). The same while the facts are, from any of the three; facts that changed make a new id |
| `touches` | What the step works on: `game:NAME` (a game's folder, for the agent's and the tools' steps on a port), `row:GAME/ROW` (a quest-log row: a hand-off, a row the person plays), `tells:GAME` (the notes the person left on REFINE), `copy:JOURNEY/NAME` (one of the person's copies), `page` (the person's other steps: a card, a question), `tools` (the tool's crates) |
| `commands` | The gp commands the step's `how.cli` runs, by path: `gp mechanic show G R, then gp mechanic say G R WORDS` runs `mechanic show` and `mechanic say`. A step with no command line runs none |

The answer's `open` lists every step that can run at once, `next` first, then each other candidate, in the order of the rules, that touches nothing an earlier open one touches. Two steps that touch the same thing are never both open. The code is crates/gp-cli/src/steps.rs (`settle`, `id_of`, `commands_of`).

Two steps name the command they lead to, so a call citing them may make it: the agent's step after the person said yes on a card runs the ask's own command, and the step to fix the model of a port runs `gp model gate`.

## gp loop steps

`gp loop steps` (`--json` for the record) lists the open steps of the whole home: each game's port in the games checkout, then each of the person's copies. A step that touches something an earlier answer's open step touches (the page, which every game's cards share) is left for later, so across the home too no two open steps touch the same thing. Each is `{id, journey, subject, who, says, commands, touches, how}`. A step that touches what a step held by one of the runner's live workers touches is left out too, but for the held step itself. The JSON also has `halted` (below) and `workers`, the runner's workers' records, newest first. It reads files only, as `next` does, and gp_run runs it at once.

## gp_run runs writes as steps

With the home's `steps_required` on, gp_run (crates/gp-mcp/src/run.rs, `loop_check`) runs a call that writes, lands or publishes only when it cites an open step (its `step` argument) whose `commands` include the call's command. Reads stay free. The check (`policy::decide_step`) comes after `policy::decide` and before the ask cards and the net for writes that change what is already there, which work as before for a call that passes it. A call with no step, a step not open now (the facts it was decided from changed) or a step that runs another command is refused with `needs: "step"`, `open_steps`, and a sentence like this one:

```text
gp corpus import writes, lands or publishes, so gp_run runs it only as a step the loop gave: call again with step set to the id of an open step whose commands include corpus import. Open steps that run it: s3f9a2c1d0b4e (agent, mygame): Your agent can add 2 new clips to the game's tests (jump 1 and jump 2). Then the tools can score them, runs corpus import. All open steps: ...
```

A worker the runner starts has `GP_STEP` set to its step: its calls cite that step unasked and may cite no other, with the setting on or off.

## Settings

The home's loop settings are `$GP_HOME/loop.toml`, changed with `gp loop set` (with no flag it prints them). gp_run refuses `gp loop set`, so only the person changes them, in their own terminal.

| Setting | Flag | Default | What |
|---|---|---|---|
| `steps_required` | `--steps-required on\|off` | off | gp_run runs writes, landings and publishes only as open steps |
| `runner.on` | `--runner on\|off` | off | gp serve's runner starts workers |
| `runner.max_workers` | `--max-workers N` | 2 | The most workers at once |
| `runner.spending` | `--spending WORDS` | empty | A note on spending, shown on the page beside the runner |
| `games` | `--games NAME,NAME` (`all` clears it) | every game | The games whose quest-log items the loop works on |

`steps_required` is off by default because the tool gate publishes each landing at once to the gp every running agent uses: on by default, it would refuse the writes of agents already at work before the owner chose it. To turn it on, `gp loop set --steps-required on`; to turn it off, `gp loop set --steps-required off`.

## Wrong

The person can mark any open step wrong: WRONG beside it in STEPS on the screen page (mechanic/screen.html, which lists the open steps with who does each and its words), or `gp loop wrong STEP --why WORDS` in their own terminal. gp serve's routes are `GET /api/steps` (`{open, halted, settings}`) and `POST /api/steps/ID/wrong` with `{why}`. The code is crates/gp-cli/src/wrong.rs.

An objection is `$GP_HOME/work/loop/wrong/ID.json`: its id (`w` and 12 hex digits), `state` (`open` or `cleared`), the step as `gp loop steps` listed it, the person's `why`, `at_ms`, and `facts`, the facts the step's answer was decided from, so a test can pin the case. While an objection is open the loop is halted:

- `next` emits no steps: `open` and `also` are empty, `halted` is `{id, step, why, at_ms, says}`, and `next` is the person's, saying the loop is halted, by which step and why.
- `gp loop steps` lists none, with the same `halted`.
- gp_run runs no write as a step (with `steps_required` on). A worker that holds a step may still write to finish it, but nothing it makes lands or publishes.
- The runner starts no worker.

The rule that clears it: a commit on the tool's main branch that adds a line naming the objection's id. That line is the test or fixture that pins the rule change, so a halt ends only with a change to the rules that comes with its test, through the tool gate, which already waits for the owner's yes when an agent runs it. The tool gate checks each patch it lands and clears every open objection a line it adds names; its result then has a line "the loop resumes: the change names objection ID, which halted it". `gp loop resume ID` (run in the gameprinter checkout, `--onto` for another branch) clears one by the same rule for a commit that landed some other way or before: it looks for a commit on the branch that adds the id (`git log -S ID`) and refuses when there is none. gp_run refuses `gp loop wrong` and `gp loop resume`: only the person marks a step wrong or resumes the loop.

A program that can write the home could write a cleared objection, as it could write a yes; the halt guards against an agent that bends the loop, not one that attacks it.

## The runner

gp serve's runner (crates/gp-cli/src/serve/runner.rs) starts the workers, so no agent controls the loop at the top. It is off by default: `gp loop set --runner on` turns it on, `--max-workers N` sets how many at once (2 by default), `--spending WORDS` puts the owner's note on spending beside it on the page (the runner keeps no count of money yet), and `gp loop set --runner off` turns it off. It reads the settings on each look, so they change without a restart. Turning it off starts nothing new; workers already at work go on until they end or are stopped.

Every 5 seconds, while it is on, an agent command is set and the loop isn't halted, it reads the open steps (as `gp loop steps` gives them) and starts a worker on each open step whose `who` is the agent, that no worker has had (a step done or stopped isn't started again while the facts it was decided from hold), and that touches nothing a live worker's step touches, up to `max_workers` live at once.

| What | How |
|---|---|
| The command | The clip agent's: `gp serve --clip-agent CMD`, else `$GP_CLIP_AGENT` (serve/clip_agent.rs). Without either nothing starts, and the page says how to set one |
| Its worktree | `$GP_HOME/work/loop/trees/ID` on branch `loop/ID`, made from main of the repo `$GP_AGENT_REPO` names (else the gameprinter checkout gp serve runs from). A game's step (a quest-log item of a game's, not the tools', or the engine's step that touches `game:NAME`, such as a corpus import) gets a worktree of the games checkout instead. The brief tells it to run `eval "$(gp worker env loop-ID --shell)"` there (docs/workers.md) |
| Its games | A game's step has `GP_GAMES_DIR` set to its own worktree, and its gp MCP server gets `--worker-games` with it, so gp_run runs its commands there: what they write (corpus import's clips, a probe's saves) stays in the worktree and reaches the person's checkout only through a gate. The gates (`model gate`, `tool gate`) still run in the person's checkout, the one they land on. Any other step's gp works on the person's checkout, as before |
| Its brief | On stdin, and in `$GP_HOME/work/loop/files/ID/brief.N.txt`: the step's id, subject, words, command, commands and touches, the rules (one step, land only through the gates, a landing waits for the person's yes, stop with a report), and for a step that names `gp agent brief --run RUN --clip CLIP` (the step to fix a port's model when a clip goes wrong first) that brief's output |
| Its step | `GP_STEP` is set to the step's id, so gp_run cites it unasked and refuses any other. A `claude` command gets gp's MCP server (`files/ID/mcp.json`, with `GP_STEP` in its environment), the tools `next`, `gp_commands` and `gp_run`, file tools scoped to its worktree and a few cargo and git commands; any other command gets `GP_STEP`, `GP_WORKER`, `GP_LOOP_BRIEF`, `GP_LOOP_WORKTREE`, `GP_GP`, `GP_HOME` and `GP_GAMES_DIR` |
| Its limits | The clip agent's time limit (`GP_CLIP_AGENT_TIMEOUT`, 3600 s by default). A worker that ends with an error is started again with how its last try ended in its brief, 2 tries in all |

Each worker's record is `$GP_HOME/work/loop/workers/ID.json`: its step, `state`, `why`, `worktree`, `branch`, `log`, `brief`, `command`, `tries`, `pid` and `last` (its log's last line). The states:

| State | When |
|---|---|
| `working` | Its process runs |
| `attempting` | Its `gp agent attempt` runs (an activity record with its step) |
| `waiting for a yes` | An open ask its gp_run left is its step's (an ask made under `GP_STEP` keeps it as `step`) |
| `retrying` | A try ended with an error and the next starts in a few seconds |
| `done` | It ended its step (exit 0) |
| `stopped` | The person stopped it, its tries or its time ran out, it couldn't start, or it ended while gp serve wasn't running |

A live worker's step stays citable by its gp_run calls though the facts it was decided from change as it works (`gp_mcp::steps::held`). Its results go back through the gates, and the engine recomputes the steps from the files on the runner's next look, so a landing shows as new steps. STEPS on the screen page lists the workers under the open steps: each with its state, its step, why and its last line, and STOP (`POST /api/loop/workers/ID/stop`), which ends it and everything it started (its own process group). The runner's line says whether it is on, how many are at work of how many, and the spending note.

A halt stops new starts; workers already running finish their step, but with `steps_required` on nothing they make lands or publishes until the halt clears.

## Quest-log items are steps

Each quest-log row of a game the loop works on (`games`) whose status is `next` or `agent working` is an open step of the agent's, after the engine's own, in the log's order: "Work on ROW: NOTE", touching `row:GAME/ROW` (and `tools` for a row marked `gp mechanic queue GAME ROW --repo tools`), running the item's own record commands (`mechanic show`, `start`, `busy`, `log`, `say`, `queue`, `probe`), `clip presses`, `corpus import` and `model gate`, and `*`: any plain write (gp picture, corpus score, code listing and the other checks that keep files), while a landing, a publish or a spend has to be named. A row whose talk ends with the person's turn (they answered or asked) is a step too, whatever its status but done, and its step says what they wrote. Its id is a hash of the game, the row, its status, its note and its talk's length, so a worker that ends with the row still open isn't started on it again until the row's status or note changes (a row reopened with new words is a new step). The runner gives a row's worker a worktree of the games checkout (of gameprinter for a tools row), and its brief says to keep the row's status line (`gp mechanic busy`) and log up to date on the page, land through the gate, and mark the row done or asking you. A row asking the person to judge something they can see comes with the clip that shows it (`--clip N --frames A-B --look THEME`), which the row and its card set up on the screens with LOOK AT IT (mechanic-page.md, The screen).

A row whose item is in the game's triage.toml and that no triage has tried (no `triage` on its quest-log row, not done, not out of scope) is a step of the tools' before the agent works on it: `gp mechanic triage GAME ROW...` with just those rows, so a row added after the first triage is tried too (next.rs, the tools' free work).

## The loop proposes rows

What the loop finds becomes quest-log rows on its own, with no card (the owner, 2026-11-19). `gp mechanic propose GAME ROW --what WORDS --from WHERE` adds the row as next, its note "Proposed by the loop: WHAT (from WHERE)", with `by = "loop"` and `from` on it; a row the quest log has, whatever its status, is left as it is, so a row the person finished or put out of scope stays so (crates/gp-cli/src/mechanic/propose.rs).

- The picture check: `gp picture GAME --run RUN --propose` proposes a row for each cause it found on 2 frames or more, but flicker, the `--top` of each group, named `picture-GROUP-DETAIL`, from the clip, frame and box it was first seen on.
- A worker: a problem it finds that isn't its own item's it proposes with its test, `--goal TEST` (and `--near`, `--start`, `--where`, `--do`, `--stop`), in the same call. The test is checked as the triage reads it and goes in the game's triage.toml as an `[[item]]` for the row (once), so the row is a step of the tools' triage before an agent or the person works on it (above). A test the triage can't read writes nothing. The worker's brief says so.

## The runner runs a yes

While the runner is on, a YES on a card runs that exact call at once: the runner takes the yes (the ask is marked used, with `ran` {state: running}), runs `gp --json COMMAND ARGV` with the home and the games checkout, and records landed, passed or failed with its last lines. The card shows each, for 15 minutes. A later gp_run with that ask's id is told what the runner did instead of running it again. A yes that puts a held change back stays gp_run's.

## No runner, no writes

Each gp serve's runner writes a heartbeat of its own every look (`$GP_HOME/work/loop/runner-PID.json`, so two servers on one home don't write over each other; the runner goes when any is fresh with a worker command): its process, the time, whether a worker command is set). While no runner goes (no fresh heartbeat from a live gp serve, the runner off, or no worker command), every gp command run from outside the loop prints a banner saying so, and one that writes (anything but a read in the policy table) is refused with exit 3; gp_run refuses a write, a landing or a publish the same way. `gp loop`, `gp serve`, `gp mcp`, `gp tool` and `gp worker` are exempt, and so is a gp a gp started (serve's children, a gate's checks, gp_run's commands: an ancestor process is a gp), a worker (`GP_STEP`), CI, and a home in the temp folder (tests).

## A worker writes through gp_run

A worker's shell runs gp's reads, but a gp command that writes, lands or publishes from it (`GP_STEP` set and the parent process not a gp) is refused with exit 3: the worker runs it through gp_run, where its step and the person's yes hold. A gate run from a shell asks no one, so this is what keeps a worker's landing on the person's card. When a quest-log item's worker ends, the runner puts its report in the item's log (`gp mechanic log`), so the page says what it did and what is left. If the row still says agent working then, no agent is on it any more: the runner sets it to asking you, with the report's questions as its note, when the report asks the person something, else to next, with what the report says is left.

## The scoreboard

A game's scoreboard is a few lines at the top of the screen page and at the end of `gp loop health`: one for each part of the game the owner works on now, each measured on one clip of the corpus. Each line has:

| Column | What |
|---|---|
| GAME STATE | The gate's straight-through match for the clip (`straight.pct` in `$GP_HOME/work/refine/GAME/status.json`), read as it is now |
| BACKGROUND, SPRITES | Frames matched of the clip's frames, from `gp picture GAME --run GAME --clip ID` (its `clips[ID].screen`) |
| APPROVED | The looks the owner approved for the part: none until approved looks are kept |

Beside each number is how it moved since the measure before: up, down or same.

The parts are the game's own file, `GAME/scoreboard.toml` in the games checkout; a game without one has no scoreboard. Each `[[line]]` has a `name` and the `clip` it is measured on, or a `level` (the corpus clips' `level` field), whose first clip in corpus.toml is taken:

```toml
[[line]]
name = "level 1"
clip = "mygame.l1.001"

[[line]]
name = "the bonus round"
level = "bonus"

[[line]]
name = "from power-on on its own draws"
clip = "mygame.poweron.001"
own_draws = true
```

A line with `own_draws = true` measures how far the model plays with its own random generator: `gp clip score GAME CLIP --own-draws` scores the clip with none of the original's random draws handed over (docs/formats/draws.md), and the line shows its frames together straight through (OWN DRAWS, in place of BACKGROUND and SPRITES). Its GAME STATE is still the gate's, with the original's draws, so the two side by side say how much of the gap is the random draws. It is kept apart from a picture line on the same clip, under `CLIP own draws`.

A picture check takes about a minute a clip, so gp serve measures in the background: every 30 seconds it looks for lines out of date and measures them one at a time with the published gp (`$GP_HOME/bin/gp`). A line is out of date when its key changes: the model the gate last landed (`model.hash` in the status), the clip's recipe and its presses files, or the gp that measures (a tool landing). What it measured is kept in `$GP_HOME/work/scoreboard/GAME.json` by clip, with the measure before, and a failed measure keeps the numbers it had and is tried again after 10 minutes. `gp loop health` and the page (`GET /api/scoreboard/{game}`) only read it; a line not yet measured says so, and one out of date says gp serve measures it again. The code is crates/gp-cli/src/scoreboard.rs.

## What is left of #43

- A failed gate attempt going back to the worker with the gate's score as a new brief, and a step that fails N times becoming a problem row for the owner (part 6). Today a worker that ends with an error is tried again once with its exit and last line.
- Watching each worker's transcript and stopping one that runs a command outside its step, writes without a plan, or does by hand what gp has a tool for (part 7, issue #41).
- A step waiting on another (a picture fix waiting on a tool change) held until that one lands (part 9).
- A gate failure after another worker's landing handed back as a retry with the new tip (part 11).
- A spending cap the runner counts against, and a machine-load ceiling from `gp worker env`'s load and cores (part 12).
- Steps that touch `tools` (a change to the tool's crates) and the page: the vocabulary is there, but `next` emits no step of the tool's own yet.
- The model-fix step's `gp model gate` and the agent's step after a yes are the only landings a step names; other journeys' steps (a theme, a new game) name few commands, so with `steps_required` on most of their writes are refused until their steps say what they run.
