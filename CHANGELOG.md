# Changelog

Each release of gp has a section here, `## gp VERSION` (a date may follow), and its GitHub release's notes are that section. A release is refused before it is cut when its section is missing ([docs/release.md](docs/release.md)). Add the next one in a commit to main before `just release`.

## gp 0.1.2

New: one setup for every agent. Connect yours with `gp mcp setup` (Claude Code, Codex, Cursor and others that run MCP servers), then ask it to port a ROM you own; it follows gp's steps one at a time. On Claude Code the gp plugin is a shortcut: `/plugin marketplace add lex00/gameprinter`, then `/plugin install gp@gameprinter`, then `/gp:port`. Its version is always gp's. Two people porting the same ROM get the same steps, in the same order, with the same questions.

The loop runs on its own: `gp serve` keeps the quest log, starts workers on its rows, lands their changes through the gates, and puts a card on your page when it needs your yes. When you say a look is right, it stays that way: no later change can make it look worse. Finished rows move to an archive after an hour, and `gp mechanic queue GAME --archived` lists them.

The screen page shows the original beside the model. LOOK AT IT on a card sets up its clip on both screens, rows a worker is on say AGENT WORKING, and parked rows fold away.

Made to come out the same every time:
- Workers start on the loop's own model and settings, not yours, and a port keeps the gp it started on.
- Workers end with a structured result, and a row closes on its measured target, never on how a report is worded.
- Triage counts tries instead of seconds, so a slower machine gets the same answer.
- A game's clips can be scored on scripted presses.
- Every MCP reply names the next step, and questions to you come as forms.
- `gp game new` gives a new game its scoreboard and triage plan.

Themes: a theme can restyle part of a board game's picture and keep the rest, and CLIPS plays the model beside the original in the theme you're making.

Tapper: attract mode, the demo, the GET READY screen, levels 1 to 4 and the first bonus round draw as the original does, and you've confirmed each one.

Releases now go through chant's release train, with builds for Mac (Apple silicon and Intel) and Linux (x86_64 and ARM). Windows comes in a later release; until then, build gp from source there. With Homebrew: `brew install lex00/tap/gp` (or `brew upgrade gp`).

The Mac binary is not signed with an Apple developer certificate yet. The install page says what a Mac shows and how to allow it.

## gp 0.1.1

Released by hand, before chant (tag v0.1.1).

## gp 0.1.0

Released by hand, before chant (tag v0.1.0).
