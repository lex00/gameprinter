# The loop

Issue #43. The owner, 2026-11-19: "i want the loop to run agents autonomously with no agent controlling the loop at the highest level", and "this has to support parallel work for a real dev loop". The loop is a rules engine: `next` (crates/gp-cli/src/next.rs) reads only files and emits steps, and the steps, not an agent, decide what is done next. Before this an agent read `next` and improvised, and in the owner's experience an agent that controls the loop bends it.

This page is the first phase: step ids and the set of open steps, gp_run held to them, and WRONG, which halts the loop.

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

## Wrong

The person can mark any open step wrong: WRONG beside it in STEPS on the screen page (mechanic/screen.html, which lists the open steps with who does each and its words), or `gp loop wrong STEP --why WORDS` in their own terminal. gp serve's routes are `GET /api/steps` (`{open, halted, settings}`) and `POST /api/steps/ID/wrong` with `{why}`. The code is crates/gp-cli/src/wrong.rs.

An objection is `$GP_HOME/work/loop/wrong/ID.json`: its id (`w` and 12 hex digits), `state` (`open` or `cleared`), the step as `gp loop steps` listed it, the person's `why`, `at_ms`, and `facts`, the facts the step's answer was decided from, so a test can pin the case. While an objection is open the loop is halted:

- `next` emits no steps: `open` and `also` are empty, `halted` is `{id, step, why, at_ms, says}`, and `next` is the person's, saying the loop is halted, by which step and why.
- `gp loop steps` lists none, with the same `halted`.
- gp_run runs no write as a step (with `steps_required` on). A worker that holds a step may still write to finish it, but nothing it makes lands or publishes.
- The runner starts no worker.

The rule that clears it: a commit on the tool's main branch that adds a line naming the objection's id. That line is the test or fixture that pins the rule change, so a halt ends only with a change to the rules that comes with its test, through the tool gate, which already waits for the owner's yes when an agent runs it. The tool gate checks each patch it lands and clears every open objection a line it adds names; its result then has a line "the loop resumes: the change names objection ID, which halted it". `gp loop resume ID` (run in the gameprinter checkout, `--onto` for another branch) clears one by the same rule for a commit that landed some other way or before: it looks for a commit on the branch that adds the id (`git log -S ID`) and refuses when there is none. gp_run refuses `gp loop wrong` and `gp loop resume`: only the person marks a step wrong or resumes the loop.

A program that can write the home could write a cleared objection, as it could write a yes; the halt guards against an agent that bends the loop, not one that attacks it.
