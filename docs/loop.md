# The loop

Issue #43. The owner, 2026-11-19: "i want the loop to run agents autonomously with no agent controlling the loop at the highest level", and "this has to support parallel work for a real dev loop". The loop is a rules engine: `next` (crates/gp-cli/src/next.rs) reads only files and emits steps, and the steps, not an agent, decide what is done next. Before this an agent read `next` and improvised, and in the owner's experience an agent that controls the loop bends it.

This page is the first phase: step ids and the set of open steps, and gp_run held to them.

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

`gp loop steps` (`--json` for the record) lists the open steps of the whole home: each game's port in the games checkout, then each of the person's copies. A step that touches something an earlier answer's open step touches (the page, which every game's cards share) is left for later, so across the home too no two open steps touch the same thing. Each is `{id, journey, subject, who, says, commands, touches, how}`. It reads files only, as `next` does, and gp_run runs it at once.

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

`steps_required` is off by default because the tool gate publishes each landing at once to the gp every running agent uses: on by default, it would refuse the writes of agents already at work before the owner chose it. To turn it on, `gp loop set --steps-required on`; to turn it off, `gp loop set --steps-required off`.
