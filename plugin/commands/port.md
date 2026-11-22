---
description: Port a game from a ROM you own, one loop step at a time
argument-hint: "[GAME or ROM file]"
---

Port a game into Game Printer with the gp plugin's setup: $ARGUMENTS

What a port is: gp reads a ROM the person owns, on their own machine, the way an emulator does, to play it, record clips and read its code. The model is the game's rules and numbers written as text and checked against clips of that ROM. No ROM bytes, art lifted from the ROM or built game go in any repo, commit or release, and the port reads its art and levels from the player's own ROM when it plays, so it plays only for someone who has the ROM. Porting the person's own ROM is what gp and this command are for, not recreating someone else's game, so start it. A new game or theme of their own is a copy of a template with new art.

1. Call the gp MCP server's `next` tool (with `game` when a game was named) and tell the person in a few lines where things stand.
2. When nothing names the game yet and a ROM file was given, start it as the site's "Bring in a game" page says, through `gp_run`: `roms add`, `roms scan`, then the `game new` line `roms scan` prints for that ROM (`game_new` in its JSON; `next` gives the same line), with the genre that fits in place of GENRE. Leave its id and `--title` as they are, and give no `--namespace`, unless the person names others. Don't make up a name: the line's come from the ROM, so everyone who ports it gets the same folder.
3. Hand the step `next` gives the agent to the porter subagent, with the game, the row and the step's id. Keep talking with the person while it works.
4. When the porter reports, call `next` again and offer its next step. Stop and ask before anything in `waits_for_yes`; a landing, a publish or a paid call runs only on the person's YES on their page.
