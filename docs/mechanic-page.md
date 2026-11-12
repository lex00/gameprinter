# The mechanic page

`gp mechanic start GAME [MECHANIC] --games GAMES` serves the page on
127.0.0.1 (crates/gp-cli/src/mechanic/server.rs): the game to play, record
clips of a mechanic in and hand them to your agent, and the game's model
beside the original. The page is `mechanic/` in this repo, plain HTML, CSS
and JavaScript. It came over from game-printer's page (docs/contributing-mechanics.md
there) with all three of its tabs.

## Layout

The game window is in the middle at the top. Its left margin holds REVIEW
REBUILD's clip picker (`#rule-card`) and the clip loaded (its row, scores,
length, note and first split); its right margin holds, while a clip plays
beside the rebuild, the frame it stands at, the measures the two games
disagree on there with both values, and the random draws taken so far.
Under the game are REVIEW REBUILD's bar, the line saying whose turn it is,
FREE PLAY's settings and the agent's panel. The quest log, the clips, the
conversation and the watch panel ("Your agent") are beneath. Below 1,200
pixels wide the margins are left out.

## The tabs

HELP WITH MECHANICS plays the original (gp_play.wasm, or
gp_arcade_play.wasm for an arcade game) and talks to your agent.

REVIEW REBUILD shows a clip on the original beside the game's model, in the
game window. The server plays the clip through both as `gp clip score` does
straight through, with the original's random draws, and keeps both pictures
of every compared frame beside the clip (`clips/NNN.review/`, made again
when the model, the mapping or the module changes). The three layouts are
side by side, the rebuild on top at a transparency, and the differences:
the original dimmed where the two agree and magenta where a pixel differs,
inside the part both games draw, with the count. A strip under the game
marks each frame green where the mapped state agrees and red where it
differs; a click goes there, and << DIFF and DIFF >> (or [ and ]) jump to
the difference before and after. Outlining the rebuild's entities is a
layer for any layout. A clip's ON THE MODEL button opens it here. PLAY THE
REBUILD plays the model live beside the original on the same presses.

FREE PLAY is the model alone, from its own start, with its own random
numbers; the original follows the presses unseen, so a clip recorded there
is the original's play. Its settings panel opens the model's own settings
screen inside the game (F1, the right stick's press, or the button).

The rebuild played live is the interpreter preview of the game's model
(scripts/web/build-preview.sh), built on first use into
`$GP_HOME/work/mechanic-preview/GAME/` (a few minutes the first time) and
built again in the background when the model, its mapping, profiles, hooks,
module or lifted assets, or the preview's crates, change. A game with hooks
plays its module (`--module`, or `$GP_HOME/work/modules/GAME.wasm`). The
server's pages are cross-origin isolated, since the module runs in a Worker
with a shared buffer.

An arcade game has no model yet: REVIEW REBUILD and FREE PLAY say so in
words and nothing plays.

## States and keys

The badge over the game names the state. A key not listed for a state does
nothing there. The rows are `ACTIONS` in mechanic/app.js, and
`node mechanic/actions.test.mjs` checks them (cargo test runs it).

| Tab | State (badge) | Keys and buttons |
|---|---|---|
| HELP | live (PLAYING) | Game buttons play. F5 or left trigger saves, F8 or right trigger loads. R or right bumper records. C or left bumper opens the open row's newest clip beside the rebuild |
| HELP | recording (RECORDING) | Game buttons play. Save and load as in live (loading before the start moves the start back). R or right bumper stops |
| any | review (YOUR CLIP) | A or Enter hands it off to the row in big letters, or for a run that spans a stage does the chosen answer (up and down choose; sealing is chosen). Select or Tab seals it instead. B or Esc drops it. R or right bumper records again |
| any | sending (SENDING) | Nothing until it is sent |
| any | sealed (SEALED) | A, B, Enter or Esc goes back to playing |
| HELP | agent (YOUR AGENT SAYS, or A CLIP BESIDE THE REBUILD) | Up and down choose. A or Enter answers. B replays the clip. C or left bumper puts the rebuild beside the clip or takes it away. Left and right switch the layout |
| HELP | idle (WATCHING YOUR AGENT) | After 30 s with no button: any button goes back |
| HELP | watch (WATCHING YOUR AGENT PLAY) | Nothing; it ends in the agent state with the demo's clip |
| REVIEW | refine (REVIEWING THE REBUILD) | A, Enter or Space plays and pauses. Left and right step a frame, the triggers (F5, F8) a second. [ and ] jump to the difference before and after. Up and down set the transparency. Select or Tab changes the layout. C or left bumper outlines the entities. B goes back to a second before the first split. R or right bumper plays the rebuild |
| REVIEW | live beside the rebuild (PLAYING THE REBUILD) | Game buttons play. Save and load (the rebuild starts again from the loaded moment). R records. V or left bumper changes the layout, [ and ] the transparency. Esc goes back to the review |
| FREE | free play (FREE PLAY: THE REBUILD) | Game buttons play. R records. F1 opens the settings. Esc goes back to HELP. Save and load do nothing |

U turns the original's sound off and on in every state but while typing a
note.

## Sealed runs

Select or Tab in review seals the run: it goes to
`$GP_HOME/sealed/GAME/sealed-run/clips/`, outside every session, and the
sealed-run row hears where it is kept. A run that spans a stage (the stage
`observe.toml`'s `[level]` reads changes during it) is offered for sealing
first. REVIEW REBUILD lists the sealed runs only when asked (OPEN A SEALED
RUN), and reads them without writing anything. The guard that keeps every
other tool off them, and a sealed run on the model, come with `gp sealed`.

## The decision model at the controller

`gp mechanic demo GAME MECHANIC --hint TEXT` plays from a clip's start
(`--from`, by default the newest) with Laya picking the buttons every few
frames, until the mechanic shows; the page watches it as it plays.
`gp mechanic say --judge` has Laya pick among a question's choices from the
shown clip's measured jumps. `just laya` serves Laya on 127.0.0.1:7720
(installing @receptron/laya under `$GP_HOME/work/laya`, or using the one
`LAYA_DIR` names). Both read the player's place from the game's state
mapping, so an arcade game is refused until it has one.

## Routes

| Route | What |
|---|---|
| `GET /api/clips/N.rebuild.json` | REVIEW REBUILD's clip: each compared frame's pictures, agreement, entities and differing measures, and the draws |
| `GET /api/clips/N/review.json`, `/api/clips/N/review/NAME.png` | The same review as kept |
| `GET /api/rebuild/scores` | The scoring status, `$GP_HOME/work/refine/GAME/status.json`, for the clip picker |
| `GET /api/rebuild/status`, `/profiles`, `POST /build`, `/start`, `GET /rebuild/*` | The rebuild played live |
| `POST /api/clips/stage` | Whether a clip spans a stage |
| `GET /api/sealed`, `/api/sealed/ID.log` | The sealed runs, for the person alone |
| `GET /api/clips/N.measure.json` | A clip's measurements, once `gp mechanic contacts` writes them |
| `GET /api/live` | The demo as it plays |
