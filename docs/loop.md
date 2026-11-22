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
| `steps_required` | `--steps-required on\|off` | off; on in a new home | gp_run runs writes, landings and publishes only as open steps |
| `runner.on` | `--runner on\|off` | off | gp serve's runner starts workers |
| `runner.max_workers` | `--max-workers N` | 2 | The most workers at once |
| `runner.spending` | `--spending WORDS` | empty | A note on spending, shown on the page beside the runner |
| `runner.model` | `--model NAME` (`""` for the person's own) | opus | The model every `claude` worker runs on (`--model`) |
| `runner.effort` | `--effort LEVEL` (`""` for the person's own) | high | Its effort (`--effort`) |
| `games` | `--games NAME,NAME` (`all` clears it) | every game | The games whose quest-log items the loop works on |

A `claude` worker is started the same on every machine: on `runner.model` and `runner.effort` (unless the agent command gives its own), with `--setting-sources project,local`, so the person's own settings, hooks, plugins and CLAUDE.md stay out, and with `CLAUDE_CODE_DISABLE_AUTO_MEMORY=1`, so their memory does too. It isn't `--bare`, which reads only `ANTHROPIC_API_KEY` and so fails for a person signed in by subscription.

`steps_required` is off by default because the tool gate publishes each landing at once to the gp every running agent uses: on by default, it would refuse the writes of agents already at work before the owner chose it. A new home is different, because no agent works on it yet: the first gp that reads the settings of a home whose folder is missing or empty writes its `loop.toml` with `steps_required` on, so two people starting fresh run the same steps in `next`'s order. A home in the temp folder (a test's) keeps the defaults. To turn it on, `gp loop set --steps-required on`; to turn it off, `gp loop set --steps-required off`.

## Wrong

The person can mark any open step wrong: OBJECT TO THIS STEP in STEPS on the loop's page (mechanic/screen.html?page=loop, which lists the open steps as small cards, each with who does it and one line of its words; a click on a card shows all its words, its id, OPEN THE ROW and OBJECT TO THIS STEP), or `gp loop wrong STEP --why WORDS` in their own terminal. gp serve's routes are `GET /api/steps` (`{open, halted, settings}`) and `POST /api/steps/ID/wrong` with `{why}`. The code is crates/gp-cli/src/wrong.rs.

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
| Its build folder | `$GP_HOME/work/target/loop-ID` (or under `$GP_TARGET_DIR`), its worktree's own cargo target. The runner removes it when the worker's step ends (done, stopped or waiting on its card: the card's gate builds in the gate's own tree), and on start removes those of workers that ended while no gp serve ran. `gp loop health` says PROBLEM under 20 GB free on the home's disk, with the biggest folders `gp loop clean` clears (ended workers' builds, idle `work/regenerate` folders, the score cache files older than a day in the `work/regenerate` folders still in use, the home's and the gate's home's, and with `--gate` the tool gate's builds). The runner clears those cache files itself every hour, so the gate's own cache (`work/regenerate/gate`, touched by every gate) no longer grows without limit |
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

Each quest-log row of a game the loop works on (`games`) whose status is `next` or `agent working` is an open step of the agent's, after the engine's own, in the log's order: "Work on ROW: NOTE", touching `row:GAME/ROW` (and `tools` for a row marked `gp mechanic queue GAME ROW --repo tools`), running the item's own record commands (`mechanic show`, `start`, `busy`, `log`, `say`, `queue`, `probe`), `clip presses`, `corpus import` and `model gate`, and `*`: any plain write (gp picture, corpus score, code listing and the other checks that keep files), while a landing, a publish or a spend has to be named. A row whose talk ends with the person's turn (they answered or asked) is a step too, whatever its status but done, and its step says what they wrote. Its id is a hash of the game, the row, its status, its note and its talk's length, so a worker that ends with the row still open isn't started on it again until the row's status or note changes (a row reopened with new words is a new step). The runner gives a row's worker a worktree of the games checkout (of gameprinter for a tools row), and its brief says to keep the row's status line (`gp mechanic busy`) and log up to date on the page, land through the gate, and mark the row done, or asking you for a look only (any other choice is the worker's). A row asking the person to judge something they can see comes with the clip that shows it (`--clip N --frames A-B --look THEME`), which the row and its card set up on the screens with LOOK AT IT (mechanic-page.md, The screen). A landing's card does too: `gp model gate` takes `--clip`, `--frames` and `--look` (the gate itself doesn't read them), and gp_run refuses a model gate card without a clip and frames, short of an import or a score-only run; the card, and its row while the card is open, get LOOK AT IT with that clip. It plays the card's change: the model beside the original is built from the card's patch (AFTER THIS CHANGE on its screen, BEFORE for main's), so the owner says YES or NO on what the yes would land (docs/mechanic-page.md).

A row whose item is in the game's triage.toml and that no triage has tried (no `triage` on its quest-log row, not done, not out of scope) is a step of the tools' before the agent works on it: `gp mechanic triage GAME ROW...` with just those rows, so a row added after the first triage is tried too (next.rs, the tools' free work).

## Finished rows go to the archive

The quest log shows the work going on, not its history (the owner, 2026-11-21). Once a minute gp serve's runner moves each row that has been done for an hour (its `changed_ms`), with no live worker's step touching it and no card of it still open, running or not yet settled onto the row, from `$GP_HOME/mechanics/GAME/queue.json` to `archive.json` beside it (crates/gp-cli/src/mechanic/archive.rs, serve/runner.rs `archive_finished`). The archived row keeps everything it had (note, asked, known, read, open, target, trouble) and gets `closed_ms` (its last change), `archived_ms`, `session` (its folder, which stays where it is, and how many clips and turns it holds) and `landed` (the newest landed card of the row: its id, what it did, and the commit from the gate's "PASS: landed K" line). A row that isn't done stays, whatever its age; out of scope rows that are done go too, and the corpus import's out-of-scope list and the findings (findings.rs) read the archive as well as the log.

The archive is read by asking an agent: `gp mechanic queue GAME --archived` lists it, newest closed first, and `--archived ROW` gives that row, or with words the rows whose name, note, asked words or facts hold them (`--json` for all of it). `gp mechanic show GAME ROW` still shows an archived row. A row the loop takes up again comes back out of the archive with its history rather than being made anew: `gp mechanic queue GAME ROW --status S`, a proposal of the same row seen in a new clip (one from a clip it names leaves it archived, as it leaves a done row done) and the same trouble again.

## The loop proposes rows

What the loop finds becomes quest-log rows on its own, with no card (the owner, 2026-11-19). `gp mechanic propose GAME ROW --what WORDS --from WHERE` adds the row as next, its note "Proposed by the loop: WHAT (from WHERE)", with `by = "loop"` and `from` on it; a row the quest log has, whatever its status, is left as it is, so a row the person finished or put out of scope stays so (crates/gp-cli/src/mechanic/propose.rs). `--plain SENTENCE` gives the row one sentence a person can read, with no addresses, row names or ids (`plain` on the row), which the loop's page shows on the row's card in place of the note; `gp mechanic queue GAME ROW --plain SENTENCE` sets or replaces it on any row, and it stays through later notes until another `--plain` (an empty one takes it off). A worker's brief asks for one with each status it sets. A row filed to wait, one that blocks nothing now, is proposed with `--parked WHY`: it is added in line, its note "Parked: WHY. Proposed by the loop: ...", in the one call. A propose followed by `gp mechanic queue GAME ROW --status "in line"` from outside a step is a hand fix to the loop (it files a trouble row), as a Q*bert smoke test's onboarding-later row did on 2026-11-21.

- The picture check: `gp picture GAME --run RUN --propose` proposes a row for each cause it found on 2 frames or more, but flicker, the `--top` of each group, named `picture-GROUP-DETAIL`, from the clip, frame and box it was first seen on, with the clips it is seen in as `seen_in`. When the quest log has the row already and a clip it shows the gap in isn't named there (in `seen_in`, or in an older row's note or from), the clip is added to `seen_in` and to the note ("Also seen in CLIP (from WHERE).") and a done row goes back to next, out of scope aside: a row is done only when every clip it names is fixed.
- A worker: a problem it finds that isn't its own item's it proposes with its test, `--goal TEST` (and `--near`, `--start`, `--where`, `--do`, `--stop`), in the same call. The test is checked as the triage reads it and goes in the game's triage.toml as an `[[item]]` for the row (once), so the row is a step of the tools' triage before an agent or the person works on it (above). A test the triage can't read writes nothing. The worker's brief says so.

### When the loop runs out of rows

When the runner's look finds fewer workers at work than `max_workers` and no step to start, the loop is idle (serve/runner.rs `idle`, true for half a minute after such a look). Going idle, the runner runs the trouble filer's health check at once. While idle, gp serve's measurer, after its measures, runs `gp picture GAME --run GAME --clip CLIP --top 2 --propose --line NAME` on each scoreboard line below full (background or sprites short of their frames, measured on the current key, not an `own_draws` line), once per line per measure key, so a landing that moves the line lets it look again (scoreboard.rs `find_work`). Each row it adds says the line in its note: "(from scoreboard line NAME: gp picture ...)". A gap the quest log has a row for adds the clip to that row, as above.

## The loop files its own trouble

The loop's own trouble becomes tools rows with no one noticing first (the owner, 2026-11-20). Each cause is one row named `trouble-WORDS-HASH`, next, `repo = "tools"`, `by = "loop"`, its note "Proposed by the loop: MESSAGE (from WHERE)", and a `trouble` table: `key` (the message with every word that holds a digit made `#`, so numbers, pids and ids don't split a cause), `kind`, `count`, `first_ms`, `last_ms`. The same cause again adds to `count` and leaves the note as it is (the note is in the row's step key); a done row it comes back to goes back to next, one out of scope stays so (crates/gp-cli/src/trouble.rs).

- page: each error the screen page shows the person (a clip that won't set up, a refused answer, a model that won't start or build) goes to `POST /mechanic/GAME/api/trouble` {text, where} as it is shown (`fault` in mechanic/screen.js), filed once a minute at most for the same words.
- health: a PROBLEM `gp loop health` prints in two checks a minute or more apart (a card that failed, a row marked working with no worker, a server on an old build, a gate run holding gate.lock with no step logged for 30 minutes, named by its pid: docs/gate.md, "The lock"), and again each check it lasts. gp serve's runner runs the check a minute after it starts and every 5 minutes after. The check ends with a TROUBLE line naming the open trouble rows.
- hand: gp serve started within the hour after the one before it stopped (its heartbeat, `work/loop/runner-PID.json`), unless the server that started it set `GP_SERVE_RELOADED`; and `gp mechanic queue GAME ROW --status S` run with no `GP_STEP` and no `GP_WORKER` while the runner goes.

Trouble with no game named (health, a restart) is filed under the loop's first game (loop.toml's games, else the games checkout's first). The screen page's status line says how many trouble rows are open, as a link to the first on the loop's page.

## The runner runs a yes

While the runner is on, a YES on a card runs that exact call at once: the runner takes the yes (the ask is marked used, with `ran` {state: running}), runs `gp --json COMMAND ARGV` with the home and the games checkout, and records landed, passed or failed with its last lines. The card shows each, for 15 minutes. A later gp_run with that ask's id is told what the runner did instead of running it again. A yes that puts a held change back stays gp_run's.

Every card has an answer box, open on its face, for what the person wants to say: what is wrong with the look they just watched, or what to keep. ANSWER (or Enter) sends the words alone to the card's row as the person's turn and leaves the card open. The cards on the game's page and the journey pages have the same box, sent with YES or NO. The words go with YES or NO (`POST /api/asks/ID` with `words`), stay on the ask, and are written to the card's row as the person's turn, without moving the row's status, so the runner still settles the row by the card. A NO with words sends the row to next with the words in its note, for its worker to make the fix again; a NO with none does as before. A YES with words lands the change and keeps the words in the row's note. gp_run's reply to a call whose card was answered quotes the words.

## gp serve moves onto a new gp

When a gate publishes another gp (`$GP_HOME/bin/gp` points at a new build), gp serve moves itself onto it (`serve/reload.rs`). From then no worker and no clip keeping starts. The person's yeses still run, since the workers' steps can take half an hour; only in the last moment before the exec does a yes wait, and the new build takes it up. Workers keep running across the move: on unix a worker's process is started on its own (a short `sh` starts it in the background with its brief on stdin and writes its pid to `files/ID/N.pid`, and its exit code goes to `files/ID/N.exit` when it ends; its record names both and its process group), so it isn't gp serve's child, and the new build's runner takes up each live record whose process runs or whose exit is written, watching it as if it had started it (its time, its stop, its next try after a failed one). The move waits only for a worker gp serve has in hand (making its worktree, between tries, writing its report), and holds the workers from its last look to the exec. Once none is in hand, the yeses it runs have written their result, the background jobs and the preview's build have ended and no request is being answered, the process becomes the new build (exec) with its arguments, on the port it listens on, and the page's polls reconnect a moment later. `--rom` is left out, so the set the page's picker chose last stays. While it waits the runner's status and `gp loop health` say what for. Each start writes `$GP_HOME/work/loop/serve/PID.json` (its start and build), which health reads, since the process keeps its pid and ps start time across the exec. `GP_SERVE_NO_RELOAD=1` keeps a server on the build it started on.

A game's workers don't move with it. The first worker the runner starts on a game's own work (a quest-log row of the game's, not a tools row, or the engine's step on `game:NAME`) starts the game's port run: `$GP_HOME/work/loop/runs/GAME.json` names the gp it starts on (`crate::port_run`). Every later worker on the game starts on that gp (its MCP server, `GP_GP` and the brief's commands), and the YES on a card one of them raised runs on the gp its worker ran, so a tool landing during a port doesn't change the tools under it, and two people porting one ROM run the same gp for its rows. A tools row's worker, and a tool gate's YES, run on the newest gp. The run ends when no row of the game's own is next, agent working or asking you and no worker is on it (a row in line, parked for the person or with its change on a card, holds no run, nor does a tools row); the next worker starts a new run on the newest gp. `gp loop set --move-on GAME` (or `all`) ends it at once. `gp loop health` and `gp loop set` print each run's gp and whether a newer one is published, and `gp self publish` and the tool gate keep a run's build when they prune old ones.

## No runner, no writes

Each gp serve's runner writes a heartbeat of its own every look (`$GP_HOME/work/loop/runner-PID.json`, so two servers on one home don't write over each other; the runner goes when any is fresh with a worker command): its process, the time, whether a worker command is set). While no runner goes (no fresh heartbeat from a live gp serve, the runner off, or no worker command), every gp command run from outside the loop prints a banner saying so, and one that writes (anything but a read in the policy table) is refused with exit 3; gp_run refuses a write, a landing or a publish the same way. `gp loop`, `gp serve`, `gp mcp`, `gp tool` and `gp worker` are exempt, and so is a gp a gp started (serve's children, a gate's checks, gp_run's commands: an ancestor process is a gp), a worker (`GP_STEP`), CI, and a home in the temp folder (tests).

## A worker writes through gp_run

A worker's shell runs gp's reads, but a gp command that writes, lands or publishes from it (`GP_STEP` set and the parent process not a gp) is refused with exit 3: the worker runs it through gp_run, where its step and the person's yes hold. A gate run from a shell asks no one, so this is what keeps a worker's landing on the person's card. When a quest-log item's worker ends, the runner puts its report in the item's log (`gp mechanic log`), so the page says what it did and what is left. If the row still says agent working then, no agent is on it any more: the runner sets it to next, with what the report says is left and any questions the report asks, for the next worker to decide. No row is closed on the wording of its note or its worker's report: a row with a target closes when its measure gets there, and one without closes only when its worker sets it done (`gp mechanic queue --status done`), so two runs of the same item end the same way. `gp game new` writes the game's `scoreboard.toml` and `triage.toml` (no lines and no items until the first clips), and each row it seeds says what would measure it (`measure_needed`) until a line of the scoreboard gives it a target. Only a look asks the owner: a row is asking you only with a clip and frames for them to watch, or a card for their YES. Under a step, `gp mechanic queue --status "asking you"` with neither is refused, and a card's NO or one that ran out sends its row to next.

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

Under the lines are the held-out runs: every sealed run of the game (docs/sealed.md), filed under `$GP_HOME/sealed/FOLDER/` for the game's folder name or a source ID its roms.toml lists, whichever way it was sealed (STOP AND SEAL on the screen page, the old mechanic page, or a log put there by hand). Each is named by the note it was sealed with (the part it plays, such as level 1), else `sealed run RUN`, and has its game state straight through and with resync and its longest stretch together, from `gp sealed score FOLDER RUN`. gp serve's measurer is the coordinator: it runs that command after the lines, a run at a time, when the model the gate landed, the game's hook code, the run's files or the gp change, and keeps the numbers (no frames, no presses) in the same `GAME.json` under `held`, with the score before so each says how it moved. It picks the module as for the lines: one given to gp serve, else none for a game with hooks of its own (so `gp sealed score` builds it from them). A sealed run's pictures aren't checked, since `gp picture` is one of the readers the guard refuses. With no sealed runs, the board says so and how to seal one. The model is never tuned on these runs: no worker's tool reads them, so a held-out number that moves with the corpus's says the model plays the game, not the clips.

## A row's target

The owner, 2026-11-20: each quest-log row states its target as a measure and closes itself when the measure gets there; a worker doesn't decide done. A target is a line of the game's scoreboard, a column and the least it must read: `LINE state|background|sprites|own at least N`, frames matched for background, sprites and an own-draws line's own (`level 1 sprites at least 330 of 336`, the `of M` only a check), the state's percent (`the demo state at least 99.5%`). `gp mechanic queue GAME ROW --target WORDS` sets it, refused when the words aren't a measure, the scoreboard has no such line, or the line hasn't that column; `--no-target` takes it off, the person's call only (refused under `GP_STEP`). A row no line can measure says what would measure it instead: `--measure-needed WORDS`. A row with neither gets a hint from `gp mechanic queue`. Held-out runs aren't targets: a row can't aim at a number no worker may look at.

| What | How |
|---|---|
| Closing | Every 30 seconds, after it measures, gp serve's scoreboard sets done each row whose target the board meets on a current measure (not out of date, not being measured, not failed), its note "Met its target: TARGET. Now READING, measured on CLIP" (`scoreboard::close_rows`) |
| A look | After it closes rows, the scoreboard asks the owner to look at each picture line near full (background and sprites each at least 99% of the frames, on a current measure): a row `confirm-LINE` (the line's words, `the` left out: `confirm-level-1`, `confirm-demo`), asking you, `by = "loop"`, with the line's clip over all its frames in the game's own pictures, so LOOK AT IT sets it up beside the original. A row the quest log has by that name, whatever its status, is left as it is (`scoreboard::ask_looks`) |
| Done by hand | `--status done` on a row with a target is refused until the board's current measure meets it, with where the measure stands |
| A landing | A YES that lands a row with a target sets it in line with `awaiting_measure` (the time), not done. Once the line is measured after that, the row is done if it meets the target, else back to next with the reading, so the loop takes it up again |
| A wait | A row that can't go on until another lands is parked on it: `gp mechanic queue GAME ROW --status "in line" --waits-on OTHER` (OTHER of the same game, or GAME/OTHER) writes `waits_on` and `waits_since`, refused for a row neither the quest log nor its archive has, and another status or `--no-wait` ends the wait. Every minute gp serve's runner looks at each row in line that waits, and once the row it waits on has a card landed since the wait began, is done, or is archived, moves it to next with a note naming the landing (`OTHER landed (COMMIT): WHAT`) and the note it waited with, or done when its target is met on the board; a row with its own card still open or unsettled stays parked. A row parked before the field, whose note says `Waits on OTHER` or `Waits for OTHER to land` naming a row of its game, waits the same way (`mechanic::waits`, `wake_waiting` in serve/runner.rs). The quest log prints `waits on: GAME/OTHER`, and the page a WAITS ON line (owner, waits-wake-on-landing) |
| Shown | `gp mechanic queue` prints `target: TARGET: now READING` (or `met`, `not measured yet`); the quest log on the loop's page has a TARGET line, or NO MEASURE YET with what would measure it. Each row with a target that reads carries `target_line`, the line it names in lower case, and the loop's page counts a line's open rows by it in PROGRESS |
| Progress | The loop's page leads with PROGRESS, a line for each scoreboard line (owner, loop-page-progress-first): LOCKED with the day when an approved look holds the line's clip over all its frames (`locked` on `GET /api/scoreboard/GAME`: day, row, frames), READY FOR YOU while its `confirm_row` asks the person, else its background and sprites as bars, with one sentence of what the open rows that target it are doing (docs/mechanic-page.md). The endpoint's `done_ms` (when each row finished in the last day, the quest log's by `changed_ms` and the archive's by `closed_ms`) gives the count of rows done today on WHAT THE AGENTS ARE DOING |

## What is left of #43

- A failed gate attempt going back to the worker with the gate's score as a new brief, and a step that fails N times becoming a problem row for the owner (part 6). Today a worker that ends with an error is tried again once with its exit and last line.
- Watching each worker's transcript and stopping one that runs a command outside its step, writes without a plan, or does by hand what gp has a tool for (part 7, issue #41).
- A step waiting on another (a picture fix waiting on a tool change) held until that one lands (part 9): a row now waits on a row by name and goes on when it lands (A wait, above), but the runner doesn't yet hold a step whose row didn't say so.
- A gate failure after another worker's landing handed back as a retry with the new tip (part 11).
- A spending cap the runner counts against, and a machine-load ceiling from `gp worker env`'s load and cores (part 12).
- Steps that touch `tools` (a change to the tool's crates) and the page: the vocabulary is there, but `next` emits no step of the tool's own yet.
- The model-fix step's `gp model gate` and the agent's step after a yes are the only landings a step names; other journeys' steps (a theme, a new game) name few commands, so with `steps_required` on most of their writes are refused until their steps say what they run.
