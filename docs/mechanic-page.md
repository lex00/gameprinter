# The mechanic page

`gp serve` hosts the page at `http://127.0.0.1:7710/mechanic/GAME/`, and its
root opens it (crates/gp-cli/src/serve/pages.rs mounts the page's server,
crates/gp-cli/src/mechanic/server.rs). `gp mechanic start GAME [MECHANIC]`
opens its CLIPS tab, starting `gp serve` when none answers; `--own-server`
serves the page alone on 127.0.0.1:7712 as before. The page is `mechanic/`
in this repo, plain HTML, CSS and JavaScript, game-printer's page
(docs/contributing-mechanics.md there) with its tabs by phase:

- CLIPS: the quest log, and the game pane on THE ORIGINAL (play, record,
  hand off, talk: once HELP WITH MECHANICS) or ON THE MODEL (a clip on the
  original beside the model: once REVIEW REBUILD), with a line of the
  background work under it.
- REFINE: gp serve's background work (`/api/capture/GAME/background`): what
  runs now with its bars, what finished, the questions, the clips scored.
  Its parts come from `mechanic/refine/` when they are there.
  Among them THE GRID (`mechanic/refine/grid.js`, game-printer's
  GridHeat, over `/api/capture/GAME/grid` from
  `crates/gp-cli/src/serve/grid.rs`): the game's clips as a heatmap of
  the model's parts (its mechanics, entity kinds and events) by contexts
  (a level and the mechanic session a clip came from), each cell colored
  by its clips' scores on the model as it stands (matches, close,
  differs, being fixed, needs you, waiting, empty). The pointer on a cell
  says which it is; a click opens its clip on CLIPS. Under it, the empty
  cell to fill next, by game-printer's priorities (most doubt around it,
  then a context nothing covers yet), as the clip to record next.
  game-printer's grid also made that clip itself; here the clips come
  from the mechanic sessions through `gp corpus import`, so "clips in
  flight" is kept for when it does.
- PLAY: the rebuild alone with its settings (once FREE PLAY).
- THEMES: the game's themes in the person's registry and its theme kit.

The header picks the game (every game gp serve has; gp serve's root opens
the one opened last, kept in `$GP_HOME/work/serve/last-game`, else lists
them) and, for a game whose roms.toml names several sets (Tapper's tapper,
rbtapper, tapperg, sutapper and tappera; Donkey Kong's OE and JU), the set:
the ones in `$GP_ROM_DIR` are listed, and choosing one plays it, draws the
model with that set's pictures (lifted by the same asset map into
`$GP_HOME/work/assets/SOURCE@SET`) and keeps it for the game
(`$GP_HOME/mechanics/GAME/rom-set`). `gp mechanic start --rom SET|FILE` and
`gp serve --rom GAME=SET|FILE` choose it too.

The agent's log (log, pins, problems) is a panel at the right on every tab,
from AGENT'S LOG in the header or L. The tab is in the address (`?tab=`).
REFINE and THEMES take the game's place, and the games hold while they show.
`mechanic/shell.js` does the tabs; `mechanic/app.js` the game and its modes.

The old inspector's views (wave 8) are drawn by the editor's build, which
gp serve serves at `/index.html` (`just editor`): `?embed=game&run=GAME`
or `?embed=inspector&run=GAME` draws one view with no header, in the
page's colors (`editor/src/Embed.tsx`). `mechanic/details/` puts them on
the page:

- REFINE, under the background work: THE CLIP MEASURED, a corpus clip,
  free play clip or store recording of the game's ROM on the original and
  on the rebuilt game from the same moment, with the lockstep panel and
  its curves, the moments, the scenario ladder, pause, point and tell with
  the requests, and the agent's sprite edits.
- DETAILS in the header, on every tab (`?details=1`): a drawer with the
  trace inspector, a store recording's frames drawn by each view (RAM
  heatmap and watch, box selection and resolution card, OAM, nametables,
  event lanes, tiles, coverage, annotations, compare two recordings).

With no editor build, or on a server of the page's own, each says so.
The sections below keep the old names for CLIPS's two views and PLAY.

## Layout

The game window is in the middle at the top. Its left margin holds REVIEW
REBUILD's clip picker (`#rule-card`) and the clip loaded (its row, scores,
length, note and first split); its right margin holds, while a clip plays
beside the rebuild, the frame it stands at, the measures the two games
disagree on there with both values, and the random draws taken so far.
Under the game are REVIEW REBUILD's bar, the line saying whose turn it is,
FREE PLAY's settings and the agent's panel. The quest log, the clips and the
conversation are beneath, in two columns. Below 1,200
pixels wide the margins are left out.

A click on a quest log row only opens it: its clips and conversation show,
and nothing goes to the agent (`POST api/pick`). HAND TO THE AGENT on the
open row hands it over: the person's "Let's do" turn (`POST api/handoff`),
once, and the row is the agent's until it answers. A row reads WORKING ON IT
only when something is on it: a person's turn waits for the agent, or gp
serve's background work has a job or a clip agent on one of its clips (the
hub the strip under the game reads). The rows follow the session poll with
no click: turns, `queue.json` and the gate's `status.json`.

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
shown clip's measured jumps. Both ask Laya on 127.0.0.1:7720. One already
answering there is used as it is, whoever started it. Else the tool asks
`gp serve` (`POST /api/laya/start` at `$GP_SERVE_URL`, by default
127.0.0.1:7710) to start it: gp serve runs scripts/laya/serve.mjs on the
install already there (`$LAYA_DIR`, else `$GP_HOME/work/laya`, with the
weights cached in
`~/.cache/receptron-laya`), waits until it answers, and stops it when gp
serve stops; `GET /api/laya` says which. With no gp serve running the tool
starts Laya for itself and stops it when it ends. Nothing is installed by
either: with no install the error says to run `just laya`, which installs
@receptron/laya under `$GP_HOME/work/laya` and fetches the weights (about
1.7 GB) on first use. Both read the player's place from the game's state
mapping. On an arcade game (Tapper, whose mapping names the player) the
demo offers Laya the board's chords instead of the pad's: up, down, left,
right, tap, wait, coin and start (gp-explore's arcade palette), keeps the
clip as an arcade log, and plays its `--frames` without counting jumps,
which are a platformer's measure; `--judge`'s jump measures stay the NES's.

## The tools first: gp mechanic triage

The quest log is for play only the person can get. `gp mechanic triage
GAME` has the tools try each row first, and each lead (`gp mechanic leads`,
NES only), with `--budget` seconds a tool. What a row has to show is a test
on RAM in the game's package folder, `triage.toml` (its format is in
`crates/gp-cli/src/mechanic/triage.rs`): it comes to hold when a clip shows
the row. The tools, in order:

| Tool | What it tries |
|---|---|
| corpus | The corpus plan's clips, each play once, the test looked for inside each clip |
| script | The row's `try` steps (probe steps) from its `from` start |
| autoplay | Tapper's auto-player, `gp mechanic autoplay --search`, which ships inside gp (arcade; `--autoplay` or $GP_AUTOPLAY name another): once from power-on, then on from the row's near point, a save point |
| search | Seeded presses from the row's near point, the generator `gp capture dense` uses |
| demo | `gp mechanic demo` with Laya (NES), when Laya answers |

A run that shows the row is cut around the frame the test holds and saved
into the row's session as a clip with `by = "tools"` and its `tool`: a path
and the presses after it, scored like any clip. The row's `triage` says
"tools", or "you" with why each tool missed and a request: the start (a
path and frame where the row's `near` test first held, in a corpus clip or
the tools' own runs), what to do and when to stop.

The quest log shows NEEDS YOU rows first with their request and PLAY FROM
HERE, which opens the row, plays the request's path to its frame unseen,
makes that the quick save (F8 goes back to it) and starts recording into
the row: the person plays only the hard stretch. Rows the tools made are
under DONE BY THE TOOLS, collapsed, with their clips. Rows not triaged show
as before.

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
| `GET /api/paths/SHA` | A stored path's log, for PLAY FROM HERE |
