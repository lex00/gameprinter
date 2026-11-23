# Changelog

Each release of gp has a section here, `## gp VERSION` (a date may follow), and its GitHub release's notes are that section. A release is refused before it is cut when its section is missing ([docs/release.md](docs/release.md)). Add the next one in a commit to main before `just release`.

## gp 0.1.5

The loop waits less. A change to gp itself lands once its gate passes, with no card to answer; cards are for a game's looks, which only you can judge. Set `land_tools = false` under `[runner]` in `loop.toml` to get the cards back.

The model gate scores only what it has to. A patch that adds a locked look or a scoreboard line scores no clip, where before it scored the whole corpus. A landing after its own dry run passed scores nothing again, and a new gp no longer throws those passes away. A dry run of a fix made on top of fixes still waiting for your YES is tested on top of them, so when they land it lands without scoring again. Cards you approve together land in one run. Each run says how many scoring passes it made.

A gate that hangs lets go. Both gates log their steps, and one that logs nothing for 15 minutes gives up its lock; before, a hung gate could hold every landing for hours. `gp loop health` says how long ago each gate's last step was. The tool gate checks that a patch applies and is formatted before it waits for the lock.

The loop makes less work for itself: no bookkeeping rows for a status you set by hand, a worker you stop stays stopped, and `gp serve` reads its workers' state every 5 seconds, not four times a second. Scoring takes half the machine's cores.

On the page, a card in YOUR TURN wraps onto a second line, where a long one used to push its buttons off the right edge.

Also fixed: the published gp could stay on an old build after a release; a worker could say a look was locked when its save had failed; `gp mechanic say` works on a row no worker has started. The release build makes the module kit with a small tool of its own, which took 9 to 12 minutes of every build job before.

Windows builds are still to come.

## gp 0.1.4

The Intel Mac archive is back. It is built on the Apple silicon runner and checked there under Rosetta, so it no longer makes the release wait on the slow Intel runner. With Homebrew, `brew install lex00/tap/gp` works on an Intel Mac again.

Each release build prints how long every step took, in its log and its summary.

The release's build job now fails when `gp self check` skips a step, where before it passed. `just release-check ARCHIVE` runs the same check of a built archive on your own machine, and `just release-plan` prints its plan on a workstation.

A worker can't mark a row whose look you confirmed as done until the look is saved, so the page no longer says "Locked" when the save failed.

Windows builds are still to come.

## gp 0.1.3

Fixed: on an installed gp with no source checkout, `gp model gate` failed before any check, because `gp game kit` looked for files on the machine that built the release (issue 46). The kit is now made from the copy gp carries. `gp self check` uses what it carries (the kit, a module build, the page's files, the templates, the page's server) where it used to only count the files, and the release's own check runs it with the source moved away.

Fixed: a gate that couldn't land for a reason outside the patch (a git lock, uncommitted edits to the same files) used up your yes (issue 45). It now checks it can land first, and keeps the yes and tries again when it can't.

The loop's page opens on progress: one block per part of the game, saying locked, ready for you, or how close it is. YOUR TURN holds only what needs you. Everything the agents are doing is folded under one heading.

You can say what you think where you're looking: every card has an answer box open on it, every row has COMMENT on its face, and your words carry the clip and frame you were watching. LOOK AT IT on a card plays the card's change, with BEFORE for the model as it is.

Faster and less repeated work: a change is checked once, not three times; workers start from a warm build and from the fixes still awaiting an answer; rows for different parts of a game run side by side; game and tools landings no longer wait on each other. `gp loop set --hold GAME` pauses a game's own rows while its tools rows go on.

Also fixed: `gp picture` could return an empty measure with no error; a reopened row kept its old target; the page could show changes that hadn't landed. A gate run that lost its caller could hang and hold the tools lock, with every other landing waiting behind it; it now fails and frees the lock. `gp self check` works on a gp built from source, and `gp self check --as-installed` checks it the way a release is checked. A module can be built from the module kit as it is.

This release has no Intel Mac build: 0.1.2 is the last one with it, and you can build gp from source there. Windows builds are still to come.

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
