# The loop

Issue #43. The owner, 2026-11-19: "i want the loop to run agents autonomously with no agent controlling the loop at the highest level", and "this has to support parallel work for a real dev loop". The loop is a rules engine: `next` (crates/gp-cli/src/next.rs) reads only files and emits steps, and the steps, not an agent, decide what is done next. Before this an agent read `next` and improvised, and in the owner's experience an agent that controls the loop bends it.

This page is the first phase: step ids and the set of open steps.

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
