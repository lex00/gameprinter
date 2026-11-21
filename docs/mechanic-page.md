# The mechanic page

`gp serve` hosts the page at `http://127.0.0.1:7710/mechanic/GAME/`, and its
root opens it (crates/gp-cli/src/serve/pages.rs mounts the page's server,
crates/gp-cli/src/mechanic/server.rs). `gp mechanic start GAME [MECHANIC]`
opens its CLIPS tab, starting `gp serve` when none answers; `--own-server`
serves the page alone on 127.0.0.1:7712 as before (the first screen at
`/start/` is gp serve's, so there the logo opens the game's page). The page is `mechanic/`
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
- PLAY: the game's model played alone (the rebuild: the interpreter, or the game's module when it has hook code) with its settings (once FREE PLAY).
- THEMES: the game's themes in the person's registry and its theme kit.

The header picks the game (every game gp serve has; gp serve's root opens
the one opened last, kept in `$GP_HOME/work/serve/last-game`, else
[the first screen](#the-first-screen), which lists them) and, for a game whose roms.toml names several sets (Tapper's tapper,
rbtapper, tapperg, sutapper and tappera; Donkey Kong's OE and JU), the set:
the ones in `$GP_ROM_DIR` are listed, and choosing one plays it, draws the
model with that set's pictures (lifted by the same asset map into
`$GP_HOME/work/assets/SOURCE@SET`) and keeps it for the game
(`$GP_HOME/mechanics/GAME/rom-set`). `gp mechanic start --rom SET|FILE` and
`gp serve --rom GAME=SET|FILE` choose it too.

With no game in the games checkout, the first screen's PORT A GAME I OWN
leads into the conversation: what the person needs (their own copy of the
game, as a file), and what to say to their agent, with the next-step
loop's own sentences for stages 0 to 2 (crate::next). It looks again every
5 s and opens the game once the agent has brought one in. Page text names
what to press or what to ask the agent, never a gp command; commands are
left only in the standalone /refine page (refine.html, refine.js) and in
error texts from the server, both for developers.

The first time a game's page is asked for, gp serve makes its server on a
thread of its own, since that can build the game's hook code (the first
time after a new gp: 36 s for Donkey Kong). Until it is ready,
`/mechanic/GAME/` answers at once with a page that says "Getting the game
ready. This takes about half a minute the first time." in the status line
and the game pane, asks `GET /api/mechanic/GAME/ready` every second and
loads the game when it is ready. Two requests start one build. If making
it fails, the page says so in plain words, with the error to give the
agent and TRY AGAIN (`?again=1`).

Under the status line, on every tab, a card for each thing the agent waits
for the person's yes on (YOUR AGENT ASKS, with YES and NO): what it does in
plain words, and what it costs when its dry run says. It comes from the
status line's answer (`waits_for_yes`, the asks `gp_run` leaves when it
refuses a command for want of a yes, docs/mcp.md "Every command"), and a
press writes the answer (`POST /api/asks/ID`). A yes shows as YOU SAID YES
until the agent uses it, once.

The agent's proposals are cards there too, one at a time with how many
wait, each with a box for words, ACCEPT and REJECT, and PUT THE OLD ONE
BACK once accepted (docs/mcp.md "Every command"). A picture for the
person's theme (YOUR AGENT PROPOSES A PICTURE, `mechanic/picture-card.js`)
shows the picture now and the new one. A change to how the person's copy
sounds (YOUR AGENT PROPOSES A NEW SOUND, `mechanic/sound-card.js`, in the
picture card's look) has PLAY WHAT YOU HAVE and PLAY THE NEW ONE, which
play the first seconds of the song as it is and with the change, one at a
time (`GET /api/sounds/ID/old.wav`, `new.wav`); the answer goes to
`POST /api/sounds/ID`. The same card takes a sound for one role of the
person's large-art theme (a proposal of kind `role`, from `gp theme sound
propose DIR --role ROLE --wav FILE`, issue #30): it names the theme, the
role and its take, plays the role's sound as it is and the new one whole,
and says NO SOUND THERE YET, with the button off, when the theme has no
file there (a new take, or a role with no sound yet).
A sprite edit the agent proposes with `assets_edit` for the person's
copy (YOUR AGENT PROPOSES A SPRITE CHANGE, `mechanic/sprite-card.js`, in
the picture card's look; an NES copy's tiles or an arcade copy's board
sprites) shows the edit's own pictures, the sprite before and after close
up and in the recorded frames (`GET /api/sprites/ID/edit.png`,
`context.png`); the answer goes to `POST /api/sprites/ID`
(docs/sprite-editing.md, "On the page").

When the agent changes the game through `gp_run` while the page is open
(any command that writes, but the talk and the log, which redraw on their
own), `next` counts it (`changes`) and the status line's poll hears it: the
game picker is read again, THEMES redraws when it shows, a clip open ON
THE MODEL is loaded again with the clips' scores, and on PLAY and REFINE
the line says "Your agent changed the game." with RELOAD.

Point and tell (REFINE's THE CLIP MEASURED: pause, box part of the
screen, say what is wrong) posts its request as before
(`POST /api/capture/GAME/input`, kind `request`). With no capture run,
which is the usual case now, it becomes the person's turn in the talk of
the mechanic open on the page, with a `point` field (the recording, the
frame, the box, its label, the choice and what is under it). `gp mechanic
show` prints it ("[pointing at Mario in walk.001 at frame 412, box
96,120,24,24: ladder]"), the talk on CLIPS shows "(pointing at Mario,
frame 412)", and the agent answers with `gp mechanic say` as for any turn.

The agent's log (log, pins, problems) is a panel at the right on every tab,
from AGENT'S LOG in the header or L. The tab is in the address (`?tab=`).
REFINE and THEMES take the game's place, and the games hold while they show.
`mechanic/shell.js` does the tabs; `mechanic/app.js` the game and its modes.

The old inspector's views (wave 8) are drawn by the editor's build, which
gp serve serves at `/index.html` (`just editor`): `?embed=game&run=GAME`
or `?embed=inspector&run=GAME` draws one view with no header, in the
page's colors (`editor/src/Embed.tsx`, `editor/src/App.tsx`). Those two
are all that is left of the old editor: `/index.html` with no `?embed=`
sends the visitor to the page at `/`. `mechanic/details/` puts them on
the page:

- REFINE, under the background work: THE CLIP MEASURED, a corpus clip,
  free play clip or store recording of the game's ROM on the original and
  on the rebuilt game from the same moment, with the lockstep panel and
  its curves, the moments, the scenario ladder, and pause, point and tell
  (what the person says there is their turn in the mechanic's talk).
  Its free play button plays the original yourself, with save points (F5,
  F8) and clips from them: on an NES game in `gp_play.wasm`, on an arcade
  game on its board in `gp_arcade_play.wasm` (gp serve's
  `/api/play/gp_arcade_play.wasm`, or `$GP_ARCADE_PLAY_WASM`) at the
  board's 512 by 480 and 30 frames a second, keyed as a cabinet (5 a
  coin, 1 the start). The server rebuilds each save point natively on the
  same player (`serve/freeplay.rs`'s `Live`) and checks its hashes
  against the page's; an arcade clip's log is the board's
  (`gp-arcade-input 0`).
- DETAILS in the header, on every tab (`?details=1`): a drawer with the
  trace inspector, a store recording's frames drawn by each view (RAM
  heatmap and watch, box selection and resolution card, OAM, nametables,
  event lanes, tiles, coverage, annotations, compare two recordings).

With no editor build, or on a server of the page's own, each says so.
The sections below keep the old names for CLIPS's two views and PLAY.

## The first screen

`/start/` (and gp serve's root when no game or copy was opened yet) asks
what the person wants to do (issue #5). It is `mechanic/start.html` with
`mechanic/journeys/` (start.js, journey.js, tabs.js, journeys.css), served by
`crates/gp-cli/src/serve/journeys.rs`. The game page's GAME PRINTER logo
opens it. It says once, at the top, what the model is (the game's rules
as a plain-text file) and what a template is (a game's rules without its
pictures, that a new game is built on), then offers three choices:

- PORT A GAME I OWN: what to bring and what to say to the agent (above),
  or the games under way to carry on with.
- MAKE A THEME FOR A GAME: the game (a game here, or one of the person's
  new games), a name and the style in words; MAKE MY THEME runs
  `gp theme look new` into `$GP_HOME/work/themes/NAME/`, named
  `local/NAME` (NAME from the title: lowercase letters, digits and `-`).
- MAKE A NEW GAME: the templates (the open ones), and a name; MAKE MY GAME
  runs `template_fork` into `$GP_HOME/work/mcp/NAME/`.

Under them, CARRY ON lists every copy (a theme or a new game: its name,
the game or template it came from, when it last changed, its step and
whose turn it is) with CARRY ON, and every game being ported with a link
to its page. There is no delete: a copy deleted is work lost, and the
folders stay where the tools made them.

A copy opens on the same page at `/start/?copy=KIND/NAME`, at its step:
where it stands and the one next step with whose turn it is (YOU, YOUR
AGENT, THE TOOLS), the agent's cards (YES and NO, and the pictures and
sound changes it proposes with ACCEPT and REJECT), and its six steps, the
one now open.

The tabs follow the open copy (issue #5, the owner's decision). The
header's picker names what is open, a game being ported or a copy, on the
first screen's page and on the game's page alike, and the tab bar under it
shows that one's tabs (`mechanic/journeys/tabs.js`):

| Open | Tabs | It opens at |
|---|---|---|
| A port | CLIPS, REFINE, PLAY, THEMES, on the game's page at `/mechanic/GAME/`, as before | CLIPS |
| A theme | PLAY (the game in that look, in a frame of the page: for a theme of a game in the games checkout, [the screen](#the-screen) with the theme picked and the model alone; for a theme of a new game or a template, SEE IT IN THE GAME's `gp theme look play`, passed through gp serve), THEMES (the theme's journey, BESIDE THE ORIGINAL in its step 5) | THEMES |
| A new game, from nothing or a template | PLAY (the game built to play, or BUILD IT TO PLAY), STEPS (its journey) | STEPS |

CLIPS and REFINE hide for a theme and a new game: neither has an original
to match clip by clip. The tab is in the URL (`&tab=play`, `themes`,
`steps`); CARRY ON, the first screen's MAKE buttons and gp serve's root
open a copy at its first tab. Leaving PLAY stops the game in it.

The steps, what each needs and when each is the step are in
[progress.md](progress.md#copies-and-the-other-journeys); the answer is
the same one `gp progress --next --copy` and the MCP tool `next` give, so
the page and the agent agree on whose turn it is. gp serve's root opens
the copy opened last when it was opened after the last game.

Each button runs a gp command through the agent's own `gp_run` (or
`template_fork` and `build_publish`), marked as the person's, so the
policy, the agent's log and the running record are the same as for the
agent. None spends money, publishes or deletes. SEE IT IN THE GAME on a
theme of a game in the games checkout starts nothing: it answers with the
game's [screen](#the-screen) on gp serve, the theme picked and the model
alone (`/mechanic/GAME/screen.html?pictures=THEME&view=solo`), which PLAY
holds in its frame. On a theme of a new game or a template it runs
`gp theme look play --background` as before, the game opening from its own
files with no ROM; `gp theme look play` stays for the command line. The
route that passes a look-play server through (`/look/`) still answers for
those. The theme's share step
shows the share link and the words of the publish request to send by
hand. A new game's last step, Play and share, has MAKE A SHARE LINK:
`gp game share` makes the copy a package (`local/NAME`, a new version each
time), builds it with its own pictures and no ROM (build-page.sh), and
publishes it into the person's own registry on this computer
(`$GP_HOME/work/share/registry`), after `gp registry publish` has compared
it with every ROM in `$GP_ROM_DIR`; the page goes beside it
(`$GP_HOME/work/share/page`). gp serve serves the two at `/share/`, so the
step's link, `/share/p?g=local/NAME&look=ID`, plays the game with no ROM asked for.
Every theme made for the game in its look step goes in the package, offered
in Settings, and the link opens in the one changed last (`gp game share
--look` names another, or `own` for the game's own pictures); with no theme
the link has no `look`.
The step says in one sentence what it takes for other people to open it:
the page and the registry put online together, on a host that sends the
page's own headers. Nothing is sent anywhere from the page. The THEMES tab of a game stays where a ported game's themes are
fitted and reviewed; the theme journey reuses its picture cards
(`mechanic/picture-card.js`).

## Layout

Under the header, on every tab, one status line (`mechanic/next.js`) says
where the port stands, the next step and who does it, with its button
(PLAY FROM HERE, or the tab it needs), and a clue in its place when the
person seems lost. It reads `GET /api/next/GAME` every 5 seconds, the same
answer as `gp progress --next` ([what next](progress.md#what-next)), and
never covers the game. Its sentences wrap onto more lines rather than being
cut, and when the window is narrow the next step starts on a line of its
own. It keeps the tallest height it has had at the window's width, so a
poll doesn't move the game up and down.

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
with a shared buffer. The game's frame itself (the rebuild's preview, a
new game built to play, and `/share/`) goes out under the public page's
own headers, its Content-Security-Policy among them
(scripts/web/public/_headers, read by `gp_mcp::page_headers`), so a game
plays here under the same rules as on the public page.

An arcade game's model plays the same way (issue #3), keyed as a cabinet,
its bodies drawn by tile kind or, where the model names them, with its
board's own art (docs/formats/board-art.md).

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
other tool off them, and a sealed run on the model, are `gp sealed`'s
([sealed.md](sealed.md)).

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
NES only), with a budget counted in plays and tries so every machine gets
the same result: the corpus, the steps and autoplay's logs are played to
their end, and the seeded search makes `--tries` runs (300). Only the
auto-player's program has a time limit, `--budget` seconds against a hang,
and a run it cuts isn't used. What a row has to show is a test
on RAM in the game's package folder, `triage.toml` (its format is in
`crates/gp-cli/src/mechanic/triage.rs`): it comes to hold when a clip shows
the row. The tools, in order:

| Tool | What it tries |
|---|---|
| corpus | The corpus plan's clips, each play once, the test looked for inside each clip |
| script | The row's `try` steps (probe steps) from its `from` start |
| autoplay | The game's own auto-player, `gp mechanic autoplay --search`, which runs the driver the game's checks.toml `[autoplay]` names in its folder (arcade; a game without one has none; `--autoplay` or $GP_AUTOPLAY name another): once from power-on, then on from the row's near point, a save point |
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

## The screen

`/mechanic/GAME/screen.html` (mechanic/screen.html and screen.js) is one
screen for a person who talks to their agent and watches: no tabs, no
graphs. At the top, centred, THE ORIGINAL (their ROM, played by the page's
player as CLIPS plays it) and THE MODEL (the rebuild's preview in a frame
off screen, driven as CLIPS's PLAY THE MODEL drives it). The header has the
PICTURES picker (the game's own, or each of the person's themes for the
game), the view (SIDE BY SIDE; ON TOP, the model over the original at the
transparency the slider sets; DIFFERENCE, black where they match; MODEL
ONLY, the model alone at its own size, played live from its own start, as
`gp theme look play` played a theme) and RECORD. Right under the header,
when the game has a `scoreboard.toml`, is its scoreboard: a line for each
part of the game worked on, with its game state, background and sprites
matched and how each moved, read from gp serve's `GET
/api/scoreboard/GAME` every 20 seconds (docs/loop.md, The scoreboard).

Live, the person plays the original with the keyboard or a gamepad and the
model follows on the frames the original read the pad on. RECORD (or R)
starts a recording; pressing it again sends the clip to the mechanic open
in the session and says so to the agent, as CLIPS's hand-off does (`POST
api/paths`, `api/clips`, `api/talk`). While recording, STOP AND SEAL
beside it keeps the recording as a held-out run instead: the page asks
which part it plays (level 1, say), sends only the play up to its start
to the shared paths and the run itself to `api/clips` with `sealed`, so
it is kept under `$GP_HOME/sealed/GAME/sealed-run/clips/` with that name
as its note, and no row and no agent hears of it (docs/sealed.md). The
scoreboard then scores it. A clip shows both games at a frame:
the original replayed in the player's second console to the clip's start
and stepped from there, the model started at the clip's start from the
original's state (`api/rebuild/start`) with the clip's presses, seeked to
the steps it had taken by the frame. Left and right step, Space plays,
Esc goes back to live play. Either way the model starts at the frame a
lockstep from power-on would have it at, one step for each frame the
original read the pad, kept in the start cache beside the original's
snapshot: a model that counts its own frames, such as a boot that waits
so many of them, goes on with the original wherever it joins.

On the loop's page, the quest log is grouped by state, each group a heading
in words with its count: NEEDS YOU (triage asks the person, or the row is
asking them), AGENT WORKING, UP NEXT, DONE. Each note says who wrote it:
YOU WROTE for a row the person made on the page, AGENT WROTE for the
agent's; once the agent's note replaces the person's, their words stay as
YOU ASKED (`asked` in `queue.json`, kept by `gp mechanic queue`). The
sentences of a note holding RAM tests, addresses or hashes are folded under
DETAILS, which a click on the row opens. A click on a row never moves where
the next recording goes: its RECORD INTO THIS ROW does, and the line says so.

When the game's mapping scores with the original's random draws
(`score_draws` and its `[[draws]]` sites, [draws](formats/draws.md)), the
model takes them live too, so a random draw never parts the two games.
`api/rebuild/start` gives the sites (`draw_sites`, a site a line), the
player records the draws at them as the original makes each frame
(gp-arcade's draws recorder, the one `gp clip score` records through), and
the page hands each frame's draws to the model with that frame's presses.
A recording keeps the draws it saw beside the clip (`clips/NNN.draws`,
sent with `POST api/clips`), and a clip shown gives the model its draws:
the kept ones, or recorded again from the clip's replay. The page's report
says how the model took them (`draws`: taken, of, and the first desync).
Only an arcade board's player records draws so far; on the NES the model
rolls its own.

The agent drives it with the MCP tool `screen_show` (`gp mechanic screen
GAME` from the command line): `clip` and `frame`, or `live`; `pictures`;
`view`; `opacity`. Each part it leaves out stays as the page has it. The
ask goes to `$GP_HOME/work/serve/GAME/screen-ask.json` with a number that
goes up; the page reads `api/screen` every second, applies a new ask, and
reports what it shows (`POST api/screen`, kept in `screen-shown.json`):
the clip and frame, the pictures, the view, the ask it applied, any reason
it couldn't, and `by: person` once the person picked something
themselves. The tool waits up to 4 s for an open page to show the ask and
returns the report with a sentence. A page opened after an ask no page
showed yet, within 10 minutes, shows it.

The screen follows the agent's work. While `gp model refine --lockstep`
runs on the game, it writes what it works on to
`$GP_HOME/work/serve/GAME/screen-work.json` (`gp_mcp::screen::work`): the
corpus clip, its start and length, the original's frames of the window it
fits and its last line in words. `api/screen` gives it as `work`, live for
two minutes after the last write and not once the refine ended, and
`api/screen/work-clip?id=ID` gives the clip's play from power-on. The page
then shows that clip on both screens, the original beside the model,
looping the window's frames and moving with each new window; the status line
under YOUR TURN reads REFINING with the clip and frames. The person
comes first: a recording is never cut off; a game button (R records at
once) goes back to playing; it follows again only after 20 s with no
press; a clip the person opened stays until they go back to playing, one
stepped while following is held, and one the agent asked for with
`screen_show` holds a minute. A line over the screens says which it is
doing (FOLLOWING YOUR AGENT, YOU ARE PLAYING with the seconds left,
RECORDING, SHOWING YOUR CHOICE, FOLLOW IS OFF), and FOLLOW THE WORK in the
header turns it off and on (kept in the browser). The report says
`work_clip` and `follow`.

The screen is the developer's loop, the working copy as it is: a file of
the theme shown changes (`api/screen` gives each theme's newest file
time), and within two seconds the model's page is loaded again in it at
the same place. The model's files change (`api/rebuild/status` is stale),
and the page asks for a build by itself, keeps the old build playing with
one line saying the new one builds, and swaps the new one in at the same
place when `status.build` changes, saying so.

The screen page shows three things: the screens, YOUR TURN and one status
line, which ends with THE LOOP'S PAGE. Everything else is on the loop's
page, the same file at `screen.html?page=loop` (`&row=ROW` opens a row
there, as a click on a row's name in the status line or a card's FOR ROW
does): the quest log with the NEW ITEM box at its top (Enter makes a new
item, SEND comments on the open row), the steps, the workers and the
runner's switches, and the clips. The loop's page shows the screens only
while a clip is open on them. Finished rows are folded under DONE with
their count. A long sentence in YOUR TURN or the status line is cut to one
plain sentence (the question it asks, else its first), with MORE for the
rest. A row's badge comes from its status: the triage's mark (NEEDS YOU,
THE TOOLS TRY) shows only on a row that isn't done or worked on.

The status line says what is being worked on (each row marked agent
working, by which worker, when it last acted and its status line, or
MARKED WORKING, NO AGENT), then the loop: THE LOOP RUNS, THE LOOP WAITS ON
YOU (with how many things are in YOUR TURN; a click goes there), THE LOOP
IS STOPPED: THE RUNNER IS OFF, THE LOOP IS HALTED, or THE LOOP HAS NOTHING
TO DO.

YOUR TURN lists only what the person can do now,
each with the button that does it: the agent's yes cards (YES and NO), the
loop's steps for this game that are the person's and have a button (PLAY
FROM HERE: the row opens, its request's path plays to its frame, the model
starts there and RECORD records into the row, so the clip goes to the step's
row), and the rows' questions, each with an ANSWER box open beside it and
the clip it is about. A question is a worker's newest turn in the row's
talk when it asks something and waits on the person (`gp mechanic say`;
the clip it shows gets LOOK AT IT), its status line when that asks
(`gp mechanic busy`; `api/screen/clips` gives each row's `busy` and
`waiting_for`), or the note of a row asking the person. The answer goes
into the row's talk, which the waiting worker hears. With none of these it
says NOTHING NEEDS YOU.

A row that says to press YES shows only beside its card: `api/asks` gives
each card its row (`row`, GAME/ROW, from its step's worker record). Once
the card is gone the row follows it (the runner's `settle_card_rows`, for
cards that ended in the last day): landed, the row is done (Landed on your
YES: what); failed, it is next with why, so the loop takes it up again;
the person's NO, or a card that ran out unanswered, leaves the row asking
one plain question to answer. A row an agent works on waits; a done row
moves only when its note still talks of the card; only a row's newest card
moves it, once (`row_settled` on the card).

A row that asks the person to judge something they can see carries the
clip that shows it: `gp mechanic queue GAME ROW --status "asking you"
--note QUESTION --clip N --frames A-B --look THEME` writes `clip` on the
row in `queue.json` (`{"n": N}`, `{"mechanic": ROW, "n": N}` for another
row's clip, or `{"id": ID}` for a corpus clip, with `from`, `to` and
`look`; `--no-clip` takes it off, and a row set asking with no clip gets a
hint). The row in the quest log, its item in YOUR TURN, its card and its
button in NOW then have LOOK AT IT with the clip in words, and a click on
any of them (or on the row itself) sets it up on the screens: the original
beside the model, in that theme (game for the game's own pictures), at
those frames, played in a loop (one frame: stood still there). The person
judges and answers in the same item.

A row asking about a look in the game's own pictures (`look` game) also
has CONFIRM: IT LOOKS RIGHT in the quest log and in YOUR TURN. CONFIRM
(`POST /api/confirm` with the row), or a plain yes typed as the answer
("yes", "looks right", "lock it"), is the owner's yes: the row goes in line
with `confirmed`, and gp serve's runner locks it with no worker, running
`gp mechanic approve` on a worktree of the games tip and landing its
approved.toml through `gp model gate`. The row is then done with the frames
it locked (`locked`). A clip not in the corpus yet waits for the clip
keeping and is tried again every two minutes; any other failure sends the
row to next with why.

A card is one line; a click on its question opens what it does. The yeses the runner lands in turn say which
runs now and which wait (RUNNING NOW, 1 OF 3; WAITS, 2 OF 3), and one it
finished says LANDED, DONE or FAILED with its last line, then leaves a
minute later.

## Routes

| Route | What |
|---|---|
| `GET /start/`, `/start/FILE` | The first screen and its files |
| `GET /api/journeys` | The games, the templates, every copy with its step, and the copy opened last |
| `POST /api/journeys/start` | `{journey: "theme", game, game_kind?, title, style}` or `{journey: "new", template, name}`: makes the copy |
| `GET /api/journeys/KIND/NAME` | A copy's answer (steps, whose turn), its buttons' last runs and its pictures |
| `POST /api/journeys/KIND/NAME/set` | `{style}`, or `{step, done}` for a step only the person finishes |
| `GET`, `POST /api/journeys/KIND/NAME/do/WHICH` | A button's last run, or starts it: `look-check`, `look-play`, `shots`; `build`, `render`, `verify`, `undo`. `verify` (CHECK THE STAGE) runs `gp level verify` on the copy's stage, a girder or tile stage alike (Moss Hop's is one); when it refused a stage, the page says what the tools said and keeps the button, so a later gp can check it |
| `GET /api/journeys/theme/NAME/look/...` | The theme's game as SEE IT IN THE GAME serves it, passed through for PLAY's frame: the public page's headers let only its own origin hold it in a frame |
| `GET /api/journeys/KIND/NAME/file/...` | `stage.png`, `shots/...`, and `play/...` (the new game built to play) |
| `GET /api/clips/N.rebuild.json` | REVIEW REBUILD's clip: each compared frame's pictures, agreement, entities and differing measures, and the draws |
| `GET /api/clips/N/review.json`, `/api/clips/N/review/NAME.png` | The same review as kept |
| `GET /api/rebuild/scores` | The scoring status, `$GP_HOME/work/refine/GAME/status.json`, for the clip picker |
| `GET /api/rebuild/status`, `/profiles`, `POST /build`, `/start`, `GET /rebuild/*` | The rebuild played live |
| `POST /api/clips/stage` | Whether a clip spans a stage |
| `GET /api/sealed`, `/api/sealed/ID.log` | The sealed runs, for the person alone |
| `GET /api/clips/N.measure.json` | A clip's measurements, once `gp mechanic contacts` writes them |
| `GET /api/live` | The demo as it plays |
| `GET /screen.html`, `/screen.js`, `/screen.css` | [The screen](#the-screen) |
| `GET /api/screen`, `POST /api/screen` | The agent's last ask for the screen, what the page shows and each theme's newest file time; the page's report of what it shows |
| `GET /api/screen/clip?n=N&mechanic=M` | A clip's start and length, for the screen to play it |
| `GET /api/screen/work-clip?id=ID` | A corpus clip's play from power-on, its start and length, for the screen to follow a refine on it |
| `GET /api/paths/SHA` | A stored path's log, for PLAY FROM HERE |
| `POST /api/confirm` | `{mechanic}`: CONFIRM, the owner's yes to a row's look in the game's own pictures, which gp serve's runner locks |
