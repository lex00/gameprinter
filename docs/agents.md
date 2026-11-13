# Agents in the loop

Brought from game-printer's docs/agents.md, edited to gameprinter's layout. Claude works on the clip the person is watching, so the clip visibly makes progress. The background work (`gp serve`'s queue, the REFINE tab) scores the clips and tunes numbers with `gp model refine --lockstep`; when no number makes a clip match, the clip is stuck on that model. A change to how a mechanic works is made by Claude, in a git worktree of gameprinter, and every step of it is posted to the clip: what it is reading, and each attempt with the clip's score before and after, a trial preview and a keep button.

Two writers post to the same record:

| Who | Where it works | How it posts |
|---|---|---|
| The driver | The coordinator, in its own worktrees | `gp agent note` and `gp agent attempt --by driver` |
| The clip agent | Started by `gp serve` on one clip, in a worktree of its own | The same commands with `--by "clip agent"` |

A run is a game of the games checkout, named by its folder (`donkey-kong`, `tapper`). `gp agent` finds the games as `gp serve` does: `--games DIR`, else `$GP_GAMES_DIR`.

## The work record

The record is `$GP_HOME/work/serve/GAME/agent/work.jsonl`, one entry a line, appended (`crates/gp-cli/src/serve/work.rs`). Each entry says who wrote it, the clip, and when, and the page hears it on the run's event stream as `agent.note`, `agent.attempt`, `agent.approved` or `agent.done`.

| Entry | What |
|---|---|
| `note` | A progress note. With `on`, what the work is on in plain words, until another `on` or a `done` |
| `attempt` | A change tried; the attempt itself is `agent/<n>/attempt.json` |
| `approved` | The person pressed keep on an attempt |
| `done` | The work on the clip stopped, and why |
| `tell` | The person's note on a clip ("Telling the agent" below); a `note` with `reply_to` answers it |

A writer is working until its next `done`, and its line reads `Claude is working on: the barrel (x parts at frame 9). Last: found the cause in the roll routine.`

### Attempts

`gp agent attempt --run R --clip C --by "clip agent" --worktree W --model-from M --commit --title T --says S` records an attempt:

1. With `--commit`, it files `CHANGE.md` as `changes/<run>-<n>.md`, commits the worktree's changes, merges main into the branch when main has moved (a conflict is undone, recorded in the attempt's `sync`, and nothing is scored), and makes the attempt's commit: the branch's whole change against main as one commit, kept by the ref `attempts/<run>/<clip>/<n>`.
2. The model scored is the run's working copy (`$GP_HOME/work/serve/GAME/model/`, the one the refine rounds write) with the changes in `M` merged in file by file (`git merge-file`, the base being `M.base`, the copy `M` started from), so the numbers the queue tuned stay. A file whose changes conflict is taken from `M` as it is, and the attempt says so. `--model DIR` scores a folder as it is instead. The game's folder is in the games checkout, a repo of its own, which is why the model change comes as a folder beside the code's worktree rather than inside it.
3. The worktree's `gp` is built (`cargo build -p gp-cli` in the worktree, target `W/target`; `--gp BIN` uses a built one) and `gp agent score` runs with it, scoring every active clip on that model. Before is the model as it stands with the server's build, the queue's scores taken where the model is the same.
4. It builds the trial preview into `agent/<n>/preview/` with the worktree's `scripts/web/build-preview.sh` (`--no-preview` skips it). An arcade game has no preview of its model yet, and the attempt says so.
5. It records the clip's score before and after, the other active clips better and worse, and both first wrong measures (`agent/<n>/curve.json`, `agent/<n>/scores.json`).

The card shows it as `Attempt 2: the barrel rolls off the edge. Clip: 62% -> 81%; others: 3 better, none worse.`

| Command | What |
|---|---|
| `gp agent note --run R [--clip C] --by driver [--on "..."] TEXT` | A note |
| `gp agent attempt --run R --clip C --by driver --title T [--says S] [--worktree W] [--model-from M \| --model DIR] [--gp BIN] [--commit] [--shape] [--no-preview]` | An attempt, scored and previewed |
| `gp agent done --run R [--clip C] --by driver [TEXT]` | The work stopped |
| `gp agent approve --run R --attempt N` | Keep, from the command line |
| `gp agent state --run R` | The record in words |
| `gp agent score --run R --model DIR [--clip C] [--out DIR]` | Every active clip scored on a model folder with this build, as JSON |

| Endpoint | What |
|---|---|
| `GET /api/capture/{run}/agent` | The record: `working`, `attempts` (each with `line` and `keepable`), the latest entries, `clip_agent`, `clip_agents` and `fix_ready` |
| `GET /api/capture/{run}/agent/attempts/{n}` | One attempt, with every clip's score before and after |
| `GET /api/capture/{run}/agent/attempts/{n}/curve` | The attempt's first wrong measures, the one before the change as `before` |
| `GET /api/capture/{run}/agent/attempts/{n}/preview/{file}` | The trial preview |
| `POST /api/capture/{run}/agent/approve` | `{attempt}`: keep |

### Keep

Keep is offered on an attempt that makes its clip better and no active clip worse. It marks the attempt approved and appends it to `$GP_HOME/work/agents/approved.json` with the same fields as game-printer's (`run`, `clip`, `clip_name`, `attempt`, `title`, `says`, `by`, `branch`, `commit`, `squash`, `squash_ref`, `worktree`, `dirty`, `shape`, `user_ok`, `auto`, `clips`, `group`, `approved_ms`). Whether an entry is merged is read from git (`GP_AGENT_REPO`, else the gameprinter checkout), never stored: its squash ref or commit is an ancestor of main, or main holds a commit with the same patch. The driver merges an entry not on main through the tool gate and restarts `gp serve`. `gp agent integrate`, game-printer's batch merge helper, makes a branch of them for the tool gate to land ("Merging attempts" below).

## The clip agent

`gp serve` starts the clip agent on one clip (`crates/gp-cli/src/serve/clip_agent.rs`): from "let the agent fix it" in the REFINE tab's queue, from "fix this clip" on a clip stuck on the model as it stands, and by itself (the hand-off, below). Up to the run's agents at once work at the same time (2 by default, at least 1 for a start by hand).

It is off unless asked for. Nothing starts Claude unless `gp serve` has `--clip-agent COMMAND` or `GP_CLIP_AGENT` is set:

```sh
GP_CLIP_AGENT=claude gp serve
GP_CLIP_AGENT="claude --model sonnet" gp serve
gp serve --clip-agent "claude --model sonnet"
```

`off` turns it off again. game-printer's fell back to the request agent and then to `claude`; gameprinter's doesn't, so a server started without either never spends the owner's money by itself. While it is off, the queue's line says so and how to turn it on.

It works in a git worktree of gameprinter, `$GP_HOME/work/agents/<run>/<clip>`, on branch `agent/<run>/<clip>`, made from main's HEAD the first time and reused after (a reused one with no change of its own is moved up to main's HEAD), with `CARGO_TARGET_DIR` set to its own `target/`, which `gp serve` clears with `cargo clean` when the agent stops. Its copy of the run's model is `$GP_HOME/work/agents/<run>/<clip>.model/`, with `<clip>.model.base/` beside it. The repo is the one `gp serve` runs from (`GP_AGENT_REPO` names another). Its prompt, `agent/clip-agent-<ms>-<clip>.prompt.txt` in the run's folder, gives the clip, its presses as runs of the platform's buttons by frame (the NES's `a`, `b`, `select`, `start` and the pad; an arcade set's `coin`, `start1`, the stick and `button1`), its score, the group's clips and the question it is stuck on, and how to work.

With `claude` the command line is the program and its own arguments, then print mode with the `Read`, `Edit`, `Write`, `Glob`, `Grep` and `Bash` tools, Edit and Write only in the worktree and the model copy. Bash is allowed one command at a time from a short list: `gp agent`, `gp model`, `gp clip`, `gp element`, `gp code`, `gp interp`, `cargo build`/`test`/`check`/`fmt`/`clippy`, `git status`/`diff`/`log`/`show`/`add`/`commit`/`revert`/`merge`, and `ls`, `cat`, `head`, `tail`, `wc`, `sed -n`, `grep`. It is refused `rm`, `find`, `git push`, `git clean`, `git stash`, `git reset --hard` and `git commit --no-verify`. Any other command gets the prompt on stdin and `GP_CLIP_RUN`, `GP_CLIP_ID`, `GP_CLIP_WORKTREE`, `GP_CLIP_MODEL`, `GP_CLIP_PROMPT`, `GP_GP` and `GP_GAMES_DIR`. Its output goes to `agent/clip-agent-<ms>-<clip>.log`.

It stops when an attempt makes the clip match, at its limits, or when stopped. A process group of its own holds it, so stop ends the builds it started too.

| Setting | What |
|---|---|
| `gp serve --clip-agent COMMAND` or `GP_CLIP_AGENT` | The command and its arguments. Off without either |
| `GP_CLIP_AGENT_TIMEOUT` | Seconds it may take. Default 3600 |
| `GP_CLIP_AGENT_ATTEMPTS` | Attempts it may make. Default 6 |

Its line follows what it does: `Agent: starting in its own copy of the repo`, then its latest note, then `Agent: trying 'the barrel rolls off the edge' (attempt 2)` while an attempt builds and scores, and at the end `Agent: the clip matches after attempt 2. Keep it to have it merged.`, `Agent: stopped at its limit (6 attempts).`, `Agent: stopped.` or `Agent: finished after 3 attempts without a match.`

| Endpoint | What |
|---|---|
| `POST /api/capture/{run}/clips/{clip}/agent` | Starts it; answers `started`, and `why` when it didn't |
| `POST /api/capture/{run}/agent/stop` | Stops every agent on the run, or `{clip}`'s alone |
| `GET`, `POST /api/capture/{run}/background/agents` | `{agents, max}`; POST `{agents}` sets how many work at once, 0 to 8 |

Each agent's state is `agent/clip-agents/<clip>.json`: the clip, `state` (`starting`, `working`, `matched`, `limit`, `stopped`, `finished` or `failed`), `why`, the worktree, branch, model copy and log, its `slot`, the group's `clips`, `about` and `question`, `auto`, and the attempts it `kept`. The queue's state has `clip_agents` (the ones working now, each with its `row`) and `agents_idle` (why none works on the stuck clips).

### The hand-off

While the run's queue is on (`gp serve` starts with every queue paused), the server looks once a second for clips stuck on the model as it stands (a refine round wrote nothing for them). They are grouped by the measure that parts first with the slot left out (every barrel's place across is one group, `entities.barrel.x`), and each group goes to one agent, up to the run's agents at once. A group with an agent working, or whose clips an approved change not on main yet covers, isn't handed off, nor one handed off already on the model as it stands (`background/state.json`'s `handed`). A handed-off agent's attempt that makes its clip better and no active clip worse is kept by itself, written to `approved.json` with `auto` true and the group's clips; started by hand, an agent waits for keep.

## The brief

`gp agent brief --run R --clip C [--json] [--no-curve]` gives everything an agent needs to start on a clip in one call (crates/gp-cli/src/agent_brief.rs, from game-printer's). It only reads. The clip is one of the run's active clips (`gp agent clips --run R` lists them with their scores, those that don't match first) and its score is the background work's. Unless `--no-curve`, it plays the clip's lockstep once on the run's model (the working copy once there is one) and gives:

| Part | What |
|---|---|
| `measures` | Each wrong track's first wrong measure, frame and values |
| `curve` | Each wrong measure's original and rebuilt values from 3 frames before its first wrong frame to 5 after |
| `inputs` | What the original's code read to write the first wrong measure on that frame: `gp element inputs`, the 6502 writer slice on an NES game and the main Z80's on an arcade game |
| `owners` | The mechanics on the wrong measures' entities, those that write the measured field first, with their parameters as the model sets them and the line in model.toml |
| `code` | The facts the model's evidence cites for them, with each ref as file, line and the routine the line is in |
| `questions`, `tells` | The open questions of the run's working copy, and the person's open tells on the clip and its group |
| `attempts`, `agent` | The attempts so far on the clip or its group, and the clip agent on it |
| `next` | The commands to run next, whole: `gp agent note`, `gp solve`, `gp element inputs`, reading the cited code, `gp agent score`, `gp agent attempt` and `gp agent done` |

With `--json` it has a fixed shape named by `schema` ("gp agent brief 3"). The clip agent's prompt starts with it. game-printer's brief read the queue's stored curve; gameprinter's queue keeps no curve, so the brief plays the lockstep itself (seconds in a release build).

## Telling the agent

The person, or the coordinator, can tell the agent something about a clip: what is wrong, at which frame and where on the screen. A tell is a work record entry of kind `tell` written `by` "user" and given an `id` counted from 1 in the run (crates/gp-cli/src/serve/tell.rs, from game-printer's). It names the `clip` and gives the `text`. Its `frame` is the clip's and `orig_frame` the original's. The point is `at` (`[x, y]`) or `region` (`[x, y, w, h]`) in game pixels, and `base` keeps the clip's score as the background work had it. A reply is a `note` entry with `reply_to` set to the `id` of the tell it answers; once replied to, a tell is closed.

| Where the tell goes | When |
|---|---|
| The agent's inbox, `.gp-inbox/tells.txt` in its worktree | A clip agent is working on the clip, alone or as one of its group. Its prompt tells it to read the file between steps and to reply with `gp agent note --run R --clip C --by "clip agent" --reply-to ID TEXT` |
| The next prompt for the clip, and the brief | No agent is on it. The open tells for the clip and its group are in the prompt's "told you this" lines and in `gp agent brief`'s `tells` |
| The coordinator | Always: `GET /api/capture/{run}/tells` and `gp agent tells` list them, and `needs_look` names the clips with an open tell and no agent |

Each tell has a `line` for a polling script to print, as `USER TELL on tapper.l1.001 at frame 40: the mug lands short`.

| Command | What |
|---|---|
| `gp agent tell --run R --clip C --says TEXT [--frame N] [--orig-frame N] [--at X,Y] [--region X,Y,W,H]` | Adds a tell |
| `gp agent tells --run R [--open]` | The tells with their replies; `--open` for those with none |
| `gp agent note --run R --clip C --reply-to ID TEXT` | A reply |

| Endpoint | What |
|---|---|
| `POST /api/capture/{run}/clips/{clip}/tell` | `{text, frame?, orig_frame?, at?, region?}`: adds a tell; answers `tell` and `delivered` (an agent's inbox got it) |
| `GET /api/capture/{run}/tells` | `tells` with replies (`?open=1` for open ones), `needs_look` and `fix_ready` |

game-printer also credited a tell when its clip got better after it (`resolved` entries, a tally in the page's header, "fixed thanks to your note"). gameprinter's background work doesn't write those yet: a tell is open or answered.

## Merging attempts

`gp agent integrate --branch NAME [--repo DIR] [--skip N] [--dry-run] [--check]` is the coordinator's helper for the approved attempts in `$GP_HOME/work/agents/approved.json` (crates/gp-cli/src/agent_integrate.rs, from game-printer's). It runs in a clean checkout of gameprinter. It:

1. Lists the approved attempts not on main (read from git), and drops an attempt whose commit is inside a later approved attempt of the same branch. `--skip` leaves attempts out and `--dry-run` only lists.
2. Makes branch NAME from main and cherry-picks them in the order they were kept. A conflict only in append-only docs keeps both sides; a conflict in a generated `schema/*.json` takes theirs; at the first other conflict it undoes that pick, names the attempt and the files, and stops with exit 1.
3. Reads the `changes/*.md` notes the picks brought (each attempt's `CHANGE.md`: `features`, `breaking`, `roadmap`, `log` and `plugins` lines). It refuses a feature name two notes both add, and bumps a piece only when a note names it in `breaking`.
4. Builds gp from the branch and writes every schema from it (`gp self schemas schema`).
5. Adds the ROADMAP line at the top of the Decided list and a row to docs/model.md's "Changes", removes the notes, and commits.
6. Runs `gp model check` on every game of the games checkout and exits 1 if one fails.

It never merges into main or scores: the tool gate lands the branch (docs/gate.md). `--check` runs `just check` on the branch at the end (game-printer's ran its scripts/merge-check.sh, which gameprinter doesn't have).

## Loop efficiency

`gp agent bench --run R [--since T] [--json] [--log]` prints one table of how well the loop is doing (crates/gp-cli/src/agent_bench.rs, from game-printer's). It only reads the run's `background/history.jsonl` (and `ledger.jsonl` and `fidelity/history.json` where a run has them; gameprinter's queue writes neither yet), `$GP_HOME/results/score/`, and under `agent/` the attempts and the agent logs. With `--log` it also appends one dated row to the notes log of `docs/research/deterministic-inference.md` (`--note FILE` names another note).

`--since` takes a round number (`300`), a UTC time (`"2026-10-30 08:00"`, or epoch seconds or milliseconds) or an age before the last round (`2h`, `30m`). Without it the window is the whole run.

| Measure | How it is read |
|---|---|
| Wall time | First to last round of the window. Gaps over 30 minutes (the server was off) are idle and left out of work time, which every rate below uses |
| Rounds, rounds per hour | History rows in the window |
| Rounds that kept a change, by source | History rows with changes, by their `source` (or `solver`), else `fit`; an approved attempt by the clip agent counts as `LLM attempt` |
| Clips fixed per hour, by source | The rows' `fixed` counts, plus clip agent attempts whose clip went from not exact to exact (`LLM attempt`, or `user note` when the attempt answered a tell) |
| Seconds per round | Mean and median gap between consecutive rounds |
| LLM calls, and per fix | Clip agent runs (`agent/clip-agent-<ms>-<clip>.log`) in the window, over the fixes by clip agent attempts |
| Exact and fidelity at the start and end | From the ledger and the fidelity history, where the run has them |

On the same files (game-printer's run `smb-side-by-side-2`: its history, ledger, fidelity history, attempts and agent log names), gameprinter's bench gives the same JSON as game-printer's, but for the run's name.

## The rules an attempt follows

- Look for the cause in the original's code before anything else and state where it is.
- A fix is a general mechanic or an option that any game can turn on, never a game's special case.
- A change to a mechanic's shape needs the person's approval, which keep is.
- Never `rm`, never push, never `--no-verify`, never commit game data.
