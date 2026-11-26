# Agents in Game Printer

Every agent comes in the same way, through gp's MCP server. Connect yours with `gp mcp setup` in a terminal: for Claude Code, Codex, Cursor, Gemini CLI and VS Code it says whether gp is connected there and the line that connects it, and `gp mcp setup --harness codex --write` (or `cursor`, `gemini`, `vscode`, `claude`) makes the connection. Any other app that runs MCP servers takes the same entry by hand, below. In Claude Code the gp plugin is the shortcut: `/plugin marketplace add lex00/gameprinter`, then `/plugin install gp@gameprinter` (docs/plugin.md). Everyone who ports a ROM gets the same tools, instructions and prompts, so two people on one ROM get the same model.

## What a port is

gp reads a ROM the person owns, on their own machine, the way an emulator does, to play it, record clips and read its code. The model is the game's rules and numbers written as text (`model.toml`) and checked against clips of that ROM. No ROM bytes, art lifted from the ROM or built game go in any repo, commit or release, and the port reads its art and levels from the player's own ROM when it plays, so it plays only for someone who has the ROM. Porting the person's own ROM is what gp is for, not recreating someone else's game: when they ask, start it (`roms add`, `roms scan`, `game new`). A new game or theme of their own is a copy of a template with new art.

## The tools

gp's MCP server starts as a local command: the full path to `gp` (agent apps don't read the shell's PATH) with the argument `mcp`, named `gp`. `gp mcp setup` writes that entry; elsewhere add it to the app's MCP config yourself. docs/mcp.md lists its tools. The ones every session uses:

| Tool | What |
|---|---|
| `next` | Where the porting stands and the next step; call it at the start and after each step |
| `gp_commands` | Every gp command, with its arguments |
| `gp_run` | Runs one gp command; writes, landings and publishes go through it only |

## The prompts and skills

The server gives every agent the plugin's words as MCP prompts, also served as resources: `port` (what `/gp:port` says: start or go on with a port, one step at a time, with the game or ROM as `target`), `porter` (the brief for one step, to hand to a subagent or follow yourself) and one per skill. Read the skill before the work it covers. They are plain Markdown in `.claude/skills/` too:

| Skill | Prompt and resource | When |
|---|---|---|
| `.claude/skills/element-extraction/SKILL.md` | `element-extraction`, `gp://skills/element-extraction` | Adding or fixing an element or a mechanic, or a clip that differs on one |
| `.claude/skills/level-design/SKILL.md` | `level-design`, `gp://skills/level-design` | A new stage from a game's level kit |
| `.claude/skills/theme-generation/SKILL.md` | `theme-generation`, `gp://skills/theme-generation` | A new theme (art and sound) for a game |

## The rules

The server's instructions say these to every agent, and the gates hold the ones about landing whatever the agent. The Claude Code plugin adds hooks (`gp loop hook`) that hold the rest; elsewhere you keep them yourself.

1. At the start, call `next`, and when the loop's runner is on, run `gp loop health` through `gp_run`: its workers do the rows, and your job is the setup. Fix each PROBLEM it lists, a failing GitHub check of main among them. What the person reports goes into the quest log as a row (`gp mechanic queue GAME ROW --status next`, with `--repo tools` for gp's own work), and you tell them in one line what is on their page; never do a row's work or raise a row's card yourself. `gp loop hook session-start < /dev/null` prints the same state.
2. Never edit a file in a game's folder of the games checkout by hand: a hand edit lands nowhere, and blocks every landing there until it is set aside (`gp loop health` says PROBLEM while it is there, and `gp loop set-aside GAME` puts it in a stash, with the person's yes). A game changes through `model_edit`, `contract_edit` and `assets_place` on a copy, or a patch made in a worktree of the games repo and landed with `gp model gate GAME --patch FILE -m MSG` through `gp_run`.
3. A change to gameprinter itself lands only with `gp tool gate --patch FILE -m MSG`.
4. A landing, a publish or a paid call waits for the person's YES on their page. The refusal gives an `ask_id`: wait with `next` (`wait_s`), then call again with it. With no page open, a client that shows forms asks them there, and one that shows none needs `confirmed_by_person`, set only after they said yes to that exact action in the conversation. Don't stop while a yes waits to be used.
5. Ask the person only what they judge by watching, with the clip that shows it.
6. No ROM bytes, lifted art or built game in a commit, and nothing made from a ROM leaves the machine.

docs/agents.md is the full brief for the agent watching the loop, and docs/loop.md and docs/workers.md say how the loop and its workers run.
