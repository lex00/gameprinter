# Changelog

Each release of gp has a section here, `## gp VERSION` (a date may follow), and its GitHub release's notes are that section. A release is refused before it is cut when its section is missing ([docs/release.md](docs/release.md)). Add the next one in a commit to main before `just release`.

## gp 0.1.9

The loop gates workers' patches together. A worker on a row marked for the train builds the whole of its work, tests it, and ends with `gp loop hand-in`. `gp loop train` then stacks the handed-in patches on the tip, drops one that does not apply, runs one tool gate and one model gate a game, and on a failure finds the patch at fault by halves and lands the rest. `gp loop train --dry-run` shows the order and gates nothing.

The quest log is locked. Every change to it goes through one function that holds a lock beside the file and writes through a temp file and a rename, so two writers at once lose no row, and a quest log that does not parse is reported and kept in place of being emptied. Cards and worker records are changed the same way, so a yes is claimed once.

The runner has its last limits. A step whose worker fails three tries files a trouble row and waits on it. `gp loop set --day-spend` caps a day's spending and `--max-load` holds new workers on a loaded machine; `gp loop health` says when either holds the runner. The runner stops a worker that pushes, runs a gate from its shell, cites another step or edits the games checkout. The guard on steering files no longer holds a patch that only adds a permission for a new command. `gp loop health` keeps a failed GitHub run's log and names the step and tests that failed.

The tool knows no game. The recipes that checked a ported game against its ROM, and the public page's release check and deploy, moved to the games repo. gameprinter's tests read no ROM and no games checkout, and two tests hold them to it. An arcade game's own checks are a command, `gp oracle board`, run from the game's folder. The old format names in asset maps are gone (`smb-area-objects` and the rest: use the `nes-...-v1` names), and `gp gaps` takes a game's listing from its own checks.toml (`[gaps] listing`).

`gp testroms` has a `z80` suite: ZEXDOC and ZEXALL on the arcade board's Z80, through a small CP/M harness. Both pass.

`gp picture --clip SESSION:N` scans a clip just recorded in a session. The screen page has SPRITE RUNS on a corpus clip: the frames where a character is missing or extra, each a click from its first frame, with the missing ones boxed on the original.

`gp game share` takes `--registry`, `--namespace`, `--source` and `--page`, and writes a registry and a page that play the game from an https address, so a game with its own pictures has a share link that opens on someone else's computer ([docs/host-your-own.md](docs/host-your-own.md)).

Moss Hop's level is a data file in its template, read and edited by `gp level` with no ROM and no assembler.

Theme work has more commands. `gp theme look where` lists where a clip draws each picture, `gp theme look cells` answers three questions about a room's tiles, `gp theme look cut` cuts one returned picture out of its backdrop to a picture's size, and `gp theme look play --shots` takes `--frames` and `--strip`. A board's colour cycles are in the theme kit. A game's package leaves out `themes/NAME-src`, the folder that holds how a theme was made.

Also: the older mechanic page is removed, and `gp serve` has only the screen page; a scoreboard line can be marked as a test line (`confirm = false`); a clip a probe made is imported as generated; `just check-web` checks the public page with no ROM; the build has 56 fewer crates; a design for scripted scenes is in [docs/scripted-scenes.md](docs/scripted-scenes.md), with nothing of it built yet.

Windows builds are still to come.

## gp 0.1.8

Themes can do more. A room piece can change as the game plays (`room_react`), drift or slide across the screen and come round again (`room_react_drift`, `room_scroll`), and be staggered by placement or by row. A piece can be drawn under an entity's bodies (`under`), a `pic`, `timed` or `also` line can hold on named screens only (`on SCREEN`), an `also` picture can be limited to a span of frames, and a counter can be left off the screen (`counter_off`). `room_mask` lays a piece larger than a cell, a room rule ending `pick=NAME` picks the same place across rooms, and a mirrored body's `also` picture is mirrored with it. `gp theme look take` brings pictures from a theme copy into a games checkout, and `gp serve`'s screen offers the game's own themes.

The public page is drawn as an arcade, and the docs site wears the same look. The player has a full screen mode (F, or the Full screen button), a link back to the list of games, and a What is this? button that first says what the page is. Its controls are read by one module, `gp_controls.wasm`, which the screen and mechanic pages use too, and the lever holds the button for as long as the stick is pulled. A game module that stops shows its own words for why. The rewind ring holds at most 32 MB. The page counts visits with PostHog.

`gp serve` has one page. Its root and `/mechanic/GAME/` open the screen page. The model on that page plays its sound, and the board's sound no longer buzzes. The page waits while the server moves onto a new build in place of showing an error, and it follows a game's module as its hook code changes.

Cards say less and mean what they say. A gate card says at most 150 words: what you will see change, what to expect and how many files. Its middle button reads DON'T LAND IT YET. A card's LOOK AT IT is checked before the card is shown, and plays from the games checkout's commit with local changes left out. A card whose change is on main already closes as landed. A yes on `gp corpus import --write` runs in the gate's own tree.

A loop worker runs inside a wall. Its reads are limited to a few named folders, its environment is cleared, and scripts go through `gp worker confined`, which allows writes only in the worktree and no network. A run has a turn cap and a spend cap (`gp loop set --max-turns`, `--max-spend`), and the runner stops a worker that stalls, repeats itself or keeps being refused. Each run's cost, turns and refusals are recorded, and `gp loop stats` counts what a landing cost. The runner starts no worker on a row that has neither a target nor words for what would measure it. A patch that removes a test or takes assertions out fails the tool gate unless it names the test (`--removes-test`), and a change to what steers workers waits for you. `gp loop replay` scores workers by running archived rows again ([docs/worker-evals.md](docs/worker-evals.md)).

The home's disk stays clear. `gp loop clean` and the hourly sweep remove the worktrees of workers that ended a day ago, their branches, module kits and compile cache files nothing used in a day, and a replay trial's builds. The tool gate prunes an older version's builds at its start.

`gp loop health` reads GitHub's checks of main, says when a game can't be published, names the games checkout's local changes, and counts the trees and branches workers left. `gp loop set-aside GAME` puts those local changes away on your yes.

Also: a level can drop the bodies removed during play (`drop_removed`); a corpus clip whose presses name NES buttons plays on an arcade game's page; `gp registry publish` refuses a package scored against a path outside it; scripts record and cut a demo video from the screen page ([docs/demo-video.md](docs/demo-video.md)).

Windows builds are still to come.

## gp 0.1.7

The model gate stops scoring a tree it has just scored. It keeps the score of each commit it lands, and of each dry run that passes, so the next run reads the tip's score in place of measuring every clip again. On a game with 86 clips that step took about eight and a half minutes after every landing; now it takes none. The newest 32 scores are kept.

`gp package scan` no longer reads gp's own score records as ROM data. A record holds thousands of frame counts, and ten of them in a row once matched a table by chance and refused a landing that added a clip. Numbers under any other key, and every other file, are compared as before.

Windows builds are still to come.

## gp 0.1.6

The controls feel like the cabinet's. The screen reads the controls at each game frame's own time, so a quick tap or a late release lands on the frame you made it. On an arcade layout the gamepad's right stick taps when pushed any way, read by how far it has travelled. Three timing settings sit under CONTROLS' TIMING for a controller that needs them, and play has a GAME SPEED.

The screens page opens the game's own settings screen with SETTINGS or F1, and MODEL ONLY loads the model with the settings you kept. The scoreboard's table sits below YOUR TURN, so the screens stay at the top.

High scores are kept in the browser. A game that names its high score table has it saved in the browser's local storage and loaded when the game starts, so it is still there after a refresh. A setting turns this off, and the settings screen can clear the scores. A recorded clip never loads stored scores.

The gates waste less of your time. A change that passed its dry run lands before new dry runs take the gate, and among waiting runs the oldest goes first. The check of the locked looks runs its clips side by side: 16 looks went from four and a half minutes to about 20 seconds. The model gate looks at the games checkout before it starts and says first when something uncommitted is in the way. A look is locked only if the tip draws it that way, and the lock is measured again when the tip has moved.

The loop keeps its workers busy and out of each other's way. Workers start while `gp serve` waits to move onto a newer build, and they and their cards run on the newest gp. A row claims only the scoreboard line its work needs, so one row no longer holds a whole game. Rows found in one clip wait for each other, and a row can be marked as taken by hand. The scoreboard measures the parts nearest a confirm row first and the locked parts last.

The page asks you less. A row asks you about a card only while that card is on your page. The screens page starts the model on a clip that begins at power-on, and waits while `gp serve` moves onto a new build.

For porting: `gp clip feasible` says whether a person could make a clip's presses, and the scoreboard counts how many of a game's clips a person could play. `gp picture` boxes an arcade sprite where the board draws it when it wraps at the screen's edge. A hook can set up what it derives from a loaded state before the first frame is drawn, and can leave a body's sprites out of the next frame only. `gp corpus import` writes an arcade session's clips with the arcade's own button names.

Also fixed: a game's module that stops says why; `gp mechanic propose` no longer writes into the games checkout; `gp loop health` knows a published build whose files are a commit on main.

Windows builds are still to come.

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
