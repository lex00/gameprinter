# The gp plugin

Everyone who ports a ROM gets the same agent setup, so two people on one ROM get the same model and the same experience. For Claude Code that setup is one plugin, `gp`, in this repo's `plugin/` folder, released with gp: its version is gp-cli's, and a test (`loop_hook::tests::the_plugin_is_released_with_gp_and_runs_these_hooks`) fails `just check` when they differ. Other harnesses read [AGENTS.md](../AGENTS.md), which gives the same tools and rules in words.

## Install

```sh
/plugin marketplace add lex00/gameprinter
/plugin install gp@gameprinter
```

The repo root's `.claude-plugin/marketplace.json` is the marketplace; it lists the one plugin, `./plugin`. `/plugin update gp` brings the plugin to the newest release; `brew upgrade gp` does the same for gp. The session's first lines say when the two differ.

## What is in it

| Part | Where | What |
|---|---|---|
| The MCP server | `plugin/.claude-plugin/plugin.json`'s `mcpServers` | `gp mcp`, named `gp` (docs/mcp.md). Claude Code names a plugin's tools `mcp__plugin_gp_gp__NAME`; the tools are the same |
| The launcher | `plugin/bin/gp` | Finds gp, since agent apps don't read the shell's PATH: `$GP_PLUGIN_GP`, gp on PATH, `~/.gameprinter/bin/gp`, `~/.local/bin/gp`, then Homebrew's |
| The skills | `plugin/skills` | A link to `.claude/skills`: element-extraction, level-design and theme-generation |
| The porter | `plugin/agents/porter.md` | A subagent that does one step of a port the loop's way: code first, writes through `gp_run`, landings through the gates, questions only with a clip |
| `/gp:port` | `plugin/commands/port.md` | Starts or continues a port: `next`, then the step handed to the porter |
| The hooks | `plugin/hooks/hooks.json` | `gp loop hook EVENT`, below |

## The hooks

Each hook runs `gp loop hook EVENT` (crates/gp-cli/src/loop_hook.rs), which reads Claude Code's hook JSON on stdin and prints its answer. A worker the runner started (`GP_STEP` set) gets none of them: the runner scopes its tools, and its step ends with its report.

| Event | What it does |
|---|---|
| `session-start` | Puts in the session gp's and the plugin's versions; when `gp mcp` would not start in the session's folder (no templates/ to serve, or working copies inside a git work tree), why and what to install, shown to the person too, with `/mcp` to reconnect once fixed (Claude Code keeps a failed start for a while); whether the loop's runner works the rows (then the session watches the setup with `gp loop health`) or not (then it calls `next`), each yes the person gave that hasn't been used, and `gp progress --next` |
| `pre-tool-use` | Refuses Edit, Write, MultiEdit and NotebookEdit on a file in a game's folder (one holding `roms.toml`) of a games checkout (its parent is `$GP_GAMES_DIR` or a git work tree's root), and says to change it through gp's tools and `gp model gate`. A copy of a game outside a checkout is the person's to edit |
| `stop` | Refuses the first stop while a yes on the person's page hasn't been used (call `gp_run` again with its `ask_id`), or while a card raised in the last half hour waits for them (wait with `next`, or say the card is up). A second stop in a row goes, as Claude Code's `stop_hook_active` says. A runner worker's cards don't hold it |

## The loop's workers

The runner starts its workers with a setup of its own (crates/gp-cli/src/serve/runner.rs: gp's MCP server with the worker's step, tools scoped to its worktree) and in a worktree of gameprinter or the games repo, so they read the same skills from `.claude/skills` without the plugin.
