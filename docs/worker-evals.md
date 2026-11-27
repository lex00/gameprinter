# Scoring the workers

The corpus, the gates and the bench score the games. This page is how the workers are scored: a replay set made from finished rows, run on each side of a change to the brief, the tools or the model, and a routine for reading the workers' transcripts by hand. The code is `crates/gp-cli/src/replay.rs`; [loop.md](loop.md) has the runner and the brief.

Only the person runs a replay. It starts paid workers, so gp_run refuses `gp loop replay` and a loop worker's shell does too.

## The replay set

Each row in a quest log's archive that has a landed commit is a real task with a known good outcome: the row as it was first asked, the commit before its landing to start from, and a gate that says pass or fail.

```text
gp loop replay --pick 30
```

This reads every game's archive in the home and writes `$GP_HOME/work/loop/replay/set.json`: up to 30 tasks, the newest closed of each kind in turn, so the set is spread across kinds.

| Kind | Which rows |
|---|---|
| `tool` | A tools row whose landed commit changed no file of the page |
| `page` | A tools row whose landed commit changed a file under `editor/`, `scripts/web/` or `site/`, or any `.js`, `.css` or `.html` |
| `model` | A game's row |

A task keeps the row's name, its target or what would measure it, and the words it was first filed with (`asked`, the person's own, else `what` or `plain`). It never keeps the row's last note, which says what landed. A row with no measure of its own gets "the gate's dry run passes on the change". A row whose commit the checkout no longer has is left out.

Keep the set fixed while two sides are compared. `gp loop replay` with nothing else lists it.

## A replay

```text
gp loop replay GAME/ROW --trials 3 --label before
gp loop replay --all --trials 2 --label before
```

Each trial:

1. A scratch home is made under `$GP_HOME/work/loop/replay/runs/GAME-ROW/STAMP-N/home`, with the task's row in its quest log as next. The worker's cards go there, where no runner answers them. Its page is marked served by the replay's own process, so a call that needs the person's yes waits on a card and is never run on the worker's word.
2. A worktree of the task's base commit is added, detached, beside it.
3. A worker starts there with today's brief, tools, environment and limits, as the runner would start it (`brief`, `claude_args` and `worker_env` of the runner), on the agent command `--agent` names (else `$GP_CLIP_AGENT`, else `claude`), for at most an hour.
4. Everything in its tree against the base, its own commits and what it left uncommitted, is written as a patch. An empty patch fails the trial.
5. The gate's dry run judges the patch: `gp tool gate --dry-run` for a tools row, `gp model gate GAME --dry-run` for a game's, onto a branch `replay/STEP` made at the base. This is the real gate, so it waits its turn for the gate's lock behind the loop's own landings.
6. The worktree, the branch, the worker's cargo target (the scratch home's `work/target`, 3 to 4 GB) and its module builds (`work/module-build`) are removed, and the worker's record says done: its pid is the replay's own process, which goes on to the next trial. The brief, the log, the patch and the rest of the scratch home stay in the trial's folder. `gp loop clean` and gp serve's hourly sweep remove the builds a trial left (an older gp's trials kept theirs: 114 GB on 2026-11-27) once its tree is gone or no worker there holds its step: an older gp's trial records say working for as long as the replay that ran them runs.

Nothing lands. One line per trial goes to `$GP_HOME/work/loop/replay/results.jsonl`: the label, the task, pass or why not, how the run ended, and the run's turns, cost and refusals from its log's result line.

Run each task more than once. Workers differ from run to run on the same task, and a task that passes two times in three is not the same as one that passes every time.

What a replay does not copy: the scratch home has no row sessions, clips or scores, so a model row's worker has less to go on than the row's first worker had. Tool and page rows replay best. A patch made on an old base may fail the gate for a reason of today's gate (a check added since), which is the same on both sides of a comparison.

## The score

```text
gp loop replay --report
```

prints a table with a row for each label, in the order the labels first ran:

| Column | What |
|---|---|
| tasks, trials | How many tasks ran under the label, and trials in all |
| pass rate | Trials the gate's dry run passed, over trials |
| pass every trial | Tasks that passed all their trials, over tasks |
| turns, cost, refusals | The means over the trials whose log has a result line |

## Before and after a change

For a named change to the brief, the tools or the model:

1. Run the set under a label on the build before the change: `gp loop replay --all --trials 3 --label NAME-before`.
2. Land the change, or build it and run that gp.
3. Run the same set with the same trials as `NAME-after`.
4. Put the two rows of `gp loop replay --report` in the table below, with the change's commit.

| Change | Label | Tasks | Trials | Pass rate | Pass every trial | Turns | Cost | Refusals |
|---|---|---|---|---|---|---|---|---|
| (no replay has been run yet) | | | | | | | | |

The first experiment waiting for a run: a worker's MCP server lists three tools in place of 33 (issue 54). Its before side needs a gp from before that change.

## Reading the transcripts

Every worker's log is kept in `$GP_HOME/work/loop/files/STEP/N.log`, one JSON line per event, and each ended try has a line in `$GP_HOME/work/loop/runs.jsonl` (the state it ended in, its turns, cost and refusals by command). This routine is done by hand, by a person or by an agent they ask, once a week. No command does it.

1. Pick 20 tries from the last week's lines of `runs.jsonl`. Take first the ones whose `state` is `stopped`, `cut off`, `errored` or `no result`, and the ones `gp loop stats` counts as landing nothing; fill the rest with tries that ended `done`.
2. For each, open its `log` and read the assistant's text and tool calls in order, beside its brief (`brief.N.txt` in the same folder). Stop at the first thing that went wrong: the first step a careful worker would not have taken.
3. Give it one label from the list below. When none fits, add a label to the list in the same change as the count.
4. Count the labels. The commonest becomes a tools row: `gp mechanic queue GAME ROW --repo tools --note WHAT --measure-needed "the label's share of a week's sample falls"`.
5. When the row's fix lands, replay the set on both sides (above).

| Label | The first thing that went wrong |
|---|---|
| wrong tool | It did by hand, or in the shell, what a gp command or tool does |
| refused command retried | A call was refused and it made the same call again, or the same call in another spelling |
| edited the wrong file | Its change is in a file the row does not concern |
| no plan | It changed files before it had read where the row stands |
| stopped early | It ended with work left that it could have done in the run |
| ran out the clock | It was cut off by its time, turn or spend limit |
| picked its own finish | It called the row done by a measure it made up |
| followed the row's text | It took a note or a turn of the row's talk as an order |
| gate not tried | It raised a card, or stopped, with no dry run |
| redid landed work | It made again a change that was already on main or on a card |
