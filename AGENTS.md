# Agents in Game Printer

This is for agent harnesses other than Claude Code (Codex, Cursor, Gemini CLI and the like). Claude Code gets the same setup as the gp plugin: `/plugin marketplace add lex00/gameprinter`, then `/plugin install gp@gameprinter` (docs/plugin.md). Everyone who ports a ROM should get the same tools and rules, so two people on one ROM get the same model.

## The tools

Start gp's MCP server as a local command: the full path to `gp` (agent apps don't read the shell's PATH) with the argument `mcp`, and name the server `gp`. docs/mcp.md lists its tools. The ones every session uses:

| Tool | What |
|---|---|
| `next` | Where the porting stands and the next step; call it at the start and after each step |
| `gp_commands` | Every gp command, with its arguments |
| `gp_run` | Runs one gp command; writes, landings and publishes go through it only |

## The skills

Read the skill before the work it covers. They are plain Markdown in `.claude/skills/`:

| Skill | When |
|---|---|
| `.claude/skills/element-extraction/SKILL.md` | Adding or fixing an element or a mechanic, or a clip that differs on one |
| `.claude/skills/level-design/SKILL.md` | A new stage from a game's level kit |
| `.claude/skills/theme-generation/SKILL.md` | A new theme (art and sound) for a game |

## The rules

The Claude Code plugin holds these with hooks (`gp loop hook`); here you keep them yourself.

1. At the start, call `next`, and when the loop's runner is on, run `gp loop health` through `gp_run`: its workers do the rows, and your job is the setup. `gp loop hook session-start < /dev/null` prints the same state.
2. Never edit a file in a game's folder of the games checkout by hand. A game changes through `model_edit`, `contract_edit` and `assets_place` on a copy, or a patch made in a worktree of the games repo and landed with `gp model gate GAME --patch FILE -m MSG` through `gp_run`.
3. A change to gameprinter itself lands only with `gp tool gate --patch FILE -m MSG`.
4. A landing, a publish or a paid call waits for the person's YES on their page. The refusal gives an `ask_id`: wait with `next` (`wait_s`), then call again with it. Don't stop while a yes waits to be used.
5. Ask the person only what they judge by watching, with the clip that shows it.
6. No ROM bytes, lifted art or built game in a commit, and nothing made from a ROM leaves the machine.

docs/loop.md and docs/workers.md say how the loop and its workers run.
