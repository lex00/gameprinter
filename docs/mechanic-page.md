# The mechanic page

`gp serve` hosts the page at `http://127.0.0.1:7710/mechanic/GAME/`, and its
root opens it (crates/gp-cli/src/serve/pages.rs mounts the page's server,
crates/gp-cli/src/mechanic/server.rs). That address opens
[the screen](#the-screen) (`mechanic/screen.html`), the one page the owner
works on, and so does `index.html` under it. The older page, with its tabs
(CLIPS, REFINE, PLAY, THEMES), was removed on 2026-11-27 (the owner,
2026-11-26: "get rid of the older one"), and with it the files only it
loaded: `mechanic/details/`, the REFINE pane's parts in `mechanic/refine/`
and the THEMES pane, `mechanic/themes/pane.js`, with the routes that served
them. `mechanic::page_files`'s test follows what the served pages load and
fails while a file under `mechanic/` is left that none of them loads.
`gp mechanic start GAME [MECHANIC]` opens the page, starting `gp serve`
when none answers; `--own-server` serves the page alone on 127.0.0.1:7712
(the first screen at `/start/` is gp serve's, so there the logo opens the
game's page). The page is `mechanic/` in this repo, plain HTML, CSS and
JavaScript. Its pages:

| Address | Files | What |
|---|---|---|
| `/mechanic/GAME/` (`screen.html`) | screen.html, screen.js, screen.css, model-load.js, nesplayer.js, and the shared controls.js | [The screen](#the-screen): the original beside the model; with `?page=loop`, the loop's page |
| `/start/` | start.html, style.css, journeys/, picture-card.js, sound-card.js, sprite-card.js | [The first screen](#the-first-screen) and a copy's steps |
| `gp mechanic watch`'s page | watch.html, watch.js, style.css | The agent's status, pins, problems, talk and log, for a person watching an agent work |
| `/refine` | refine.html, refine.js, refine.css, style.css | The refinement view: the rule `gp rule` found and the last `gp model refine` run, for developers |

The game is the one in the address; gp serve's root opens the one opened
last, kept in `$GP_HOME/work/serve/last-game`, else the first screen,
which lists them. For a game whose roms.toml names several sets (Tapper's
tapper, rbtapper, tapperg, sutapper and tappera; Donkey Kong's OE and JU),
`gp mechanic start --rom SET|FILE` and `gp serve --rom GAME=SET|FILE`
choose the set: the screen plays it, draws the model with that set's
pictures (lifted by the same asset map into
`$GP_HOME/work/assets/SOURCE@SET`), and the choice is kept for the game
(`$GP_HOME/mechanics/GAME/rom-set`).

With no game in the games checkout, the first screen's PORT A GAME I OWN
leads into the conversation: what the person needs (their own copy of the
game, as a file), and what to say to their agent, with the next-step
loop's own sentences for stages 0 to 2 (crate::next). It looks again every
5 s and opens the game once the agent has brought one in. Page text names
what to press or what to ask the agent, never a gp command; commands are
left only in the standalone /refine page and in error texts from the
server, both for developers.

The first time a game's page is asked for, gp serve makes its server on a
thread of its own, since that can build the game's hook code (the first
time after a new gp: 36 s for Donkey Kong). Until it is ready,
`/mechanic/GAME/` answers at once with a page that says "Getting the game
ready. This takes about half a minute the first time." in the status line
and the game pane, asks `GET /api/mechanic/GAME/ready` every second and
loads the game when it is ready. Two requests start one build. If making
it fails, the page says so in plain words, with the error to give the
agent and TRY AGAIN (`?again=1`).

The agent's yes cards (the asks `gp_run` leaves when it refuses a command
for want of a yes, docs/mcp.md "Every command") are in the screen's YOUR
TURN, with YES and NO (`GET /api/asks`, `POST /api/asks/ID`). A copy's
proposals, a picture for the person's theme, a change to how their copy
sounds, and a sprite edit (`mechanic/picture-card.js`, `sound-card.js`,
`sprite-card.js`), are cards on the copy's steps on the first screen, each
with ACCEPT and REJECT. The sound card plays the first seconds of the song
as it is and with the change (`GET /api/sounds/ID/old.wav`, `new.wav`;
the answer goes to `POST /api/sounds/ID`), and takes a sound for one role
of a large-art theme (a proposal of kind `role`, from `gp theme sound
propose DIR --role ROLE --wav FILE`, issue #30). The sprite card shows the
sprite before and after close up and in the recorded frames (`GET
/api/sprites/ID/edit.png`, `context.png`; the answer goes to `POST
/api/sprites/ID`; docs/sprite-editing.md, "On the page").

The old inspector's views (wave 8) are drawn by the editor's build, which
gp serve serves at `/index.html` (`just editor`): `?embed=game&run=GAME`
or `?embed=inspector&run=GAME` draws one view with no header, in the
page's colors (`editor/src/Embed.tsx`, `editor/src/App.tsx`), and
`/index.html` with no `?embed=` sends the visitor to the page at `/`. The
older page framed them in REFINE and its DETAILS drawer; no page frames
them now. The server still takes point and tell's request (`POST
/api/capture/GAME/input`, kind `request`, which with no capture run becomes
the person's turn in the open mechanic's talk with a `point` field), which
only REFINE sent.

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

The screen page, from the top:

- The header: GAME PRINTER (to the first screen), the game's title,
  PICTURES (the game's own or a theme), the view (SIDE BY SIDE, ON TOP,
  DIFFERENCE, MODEL ONLY), MODEL's transparency slider while ON TOP shows,
  SETTINGS (the model's own settings screen), RECORD, STOP AND SEAL while
  recording, the row a recording goes to, and FOLLOW THE WORK.
- A line that says how to connect an agent, while none connected to gp mcp
  this week, and the follow line (FOLLOWING YOUR AGENT, YOU ARE PLAYING,
  RECORDING, SHOWING YOUR CHOICE, FOLLOW IS OFF).
- The two screens, centred: THE ORIGINAL and THE MODEL (the rebuild's
  preview runs in a frame off screen and is drawn here).
- While a clip shows, its bar: its name, PLAY, the frame slider and the
  frame, BEFORE on a gate card's look, and BACK TO PLAYING.
- The status line: what is worked on, by whom, whether the loop runs or
  waits on the person, and THE LOOP'S PAGE. Under it the keys, and
  CONTROLS' TIMING, folded ([below](#the-controls-timing)).
- YOUR TURN, then the game's scoreboard when it has one.

The loop's page (`screen.html?page=loop`) is the same file with
PROGRESS, YOUR TURN and WHAT THE AGENTS ARE DOING (the cards answered, the
quest log, the steps, the workers and the clips) in place of the
scoreboard; it shows the screens only while a clip is open on them. Both
are laid out in [the screen](#the-screen).

A click on a quest log row only opens it: its clips, notes and talk show,
and nothing goes to the agent. RECORD INTO THIS ROW on the open row makes
it the row the next recording goes to, and SEND puts the typed words in
its talk as the person's turn. The rows follow the session poll with no
click: turns, `queue.json` and the gate's `status.json`.

## The model

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
A server that wasn't given `--module` follows the package's hook code: every half minute it builds the module again when the code changed (`Rebuild::follow`), and the preview built next and a clip scored on the model play that one (`Rebuild::module_now`). Until 2026-11-24 it kept the module of its start, so hook code landed under a running server did nothing on its page.

An arcade game's model plays the same way (issue #3), keyed as a cabinet,
its bodies drawn by tile kind or, where the model names them, with its
board's own art (docs/formats/board-art.md).

MODEL ONLY plays the model alone at its own size, from its own start, with
its own random numbers. SETTINGS (F1, or the right stick's press) opens the
model's own settings screen inside the game, in MODEL ONLY; beside the
original the model is always the model as written.

## States and keys

The keys are read by screen.js (its keydown handler); the game's own
buttons go to the shared controls ([docs/controls.md](controls.md)), keyboard
or gamepad.

| State | Keys |
|---|---|
| Live: the person plays the original, the model follows | Game buttons play. R records; R again sends the clip to the row it goes to. F1 opens the settings screen. U turns the model's sound off and on (kept in the browser) |
| Recording | Game buttons play. R stops and sends. STOP AND SEAL keeps the run sealed instead ([sealed runs](#sealed-runs)) |
| A clip on both screens | Space or Enter plays and stands still. Left and right step a frame. Esc goes back to live play |
| Following the agent's work | As a clip. R takes the screen back and records; any other game button goes back to live play |
| The settings screen open | Its keys go to it alone |
| Typing in a box | The keys are the box's; Esc leaves it |

## Sealed runs

STOP AND SEAL while recording seals the run: the page asks which part it
plays, and it goes to `$GP_HOME/sealed/GAME/sealed-run/clips/` with that
name as its note, outside every session, and no row and no agent hears of
it. Only the play up to its start goes to the shared paths. The
scoreboard's HELD OUT lines score the sealed runs (`gp sealed score`). The guard that keeps
every other tool off them, and a sealed run on the model, are `gp sealed`'s
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
PICTURES picker (the game's own; each theme in the game's folder,
`GAME/themes/NAME` with its kit left out, published or not; each of the
person's theme copies made from the game; and each theme folder a row of
the quest log names with `--look DIR`, below), the view (SIDE BY SIDE; ON TOP, the model over the original at the
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
so many of them, goes on with the original wherever it joins. A clip
that starts at frame 0 (a whole power-on) starts the model as it opens,
with no frame of the original before it. The server moves itself onto
a new build in place, and a request sent in that moment can't reach it:
while the model builds and starts, the screen page sends such a request
again every second for up to a minute, saying it waits for the server,
and asks for the build again if the server came back having lost it
(neither built nor building); it says the model couldn't start only
when the server stays away or answers with an error. The model's own
frame is loaded again too, every 2 s for up to a minute
(mechanic/model-load.js), when it loads with no preview running in it
or sends nothing for 30 s: the preview says its first word only once
every file it imports has come. While it moves, gp
serve refuses what comes in with 503, unread: a page's call gets
`{"error": "gp serve is moving onto a newly published gp; ..."}`, which
the screen page waits through as above, and the first screen's calls
(mechanic/journeys/reach.js: a copy's steps, its PLAY and its buttons)
the same way, with a line under the tabs saying it waits; a page the
browser opens (a tab, or the game in a copy's PLAY frame) gets a line in
words that opens it again by itself a second later.

On the loop's page, the quest log is grouped by state, each group a heading
in words with its count: NEEDS YOU (triage asks the person, or the row is
asking them), AGENT WORKING (marked agent working, or a live worker of the
runner's is on the row, whose status stays next while it works), UP NEXT,
IN LINE and DONE; IN LINE and DONE stay folded until their heading is
clicked. Each row is a small card: its badge, a short title (its name in
words, a trailing hash left off), the worker on it, and one line of what
for a person: the row's `plain` sentence (`gp mechanic queue --plain`,
`gp mechanic propose --plain`, kept until another `--plain`; an empty one
takes it off), else the first sentence of what the person asked or its
note with "Proposed by the loop:", "(from ...)" and the sentences for the
tools left off. LOOK AT IT and CONFIRM are on the card when it has them. A
click on the card shows the rest: its full name, scores, target, notes,
talk, RECORD INTO THIS ROW, and a box for the person's words, open, whose
SEND puts them in the row's talk as the person's turn (what is typed is
kept through the list's redraws). Each note says who wrote it:
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
themselves. It reports the model's own word too: the step it last said it
stands at (`model_step`), of how many it was given (`model_steps`), the
step the page asked for on a clip (`model_asked`) and what it last threw
(`model_error`); the sentence says when the model isn't following the clip
(more than 4 steps behind, or no answer) or threw. The tool waits up to 4 s
for an open page to show the ask and returns the report with a sentence. A page opened after an ask no page
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

A corpus clip on the screens (a followed refine's, or a row's LOOK AT IT
on a corpus clip) has SPRITE RUNS on its bar. It lists, beside the clip,
the runs of frames where the original shows a sprite the model doesn't
draw (missing) or the model draws one the original doesn't (extra), with
their frames and the codes or kinds, from the scan `gp picture --sprites`
makes. A run clicked stands both games at its first frame, and while the
clip stands in a run the missing sprites are boxed on the original's side.
A clip with no runs says so. `api/screen/sprite-runs?id=ID` runs the scan
off the page's thread (it plays the clip on the package's scored set and
the model as it is): it answers `scanning` until the scan ends, then the
runs. The scan's lines are kept in
`$GP_HOME/work/screen-sprites/GAME/MODEL/ID.jsonl`, by the clip and the
model's version, so opening the list twice scans once and a changed model
scans again.

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
does). It leads with progress (owner, loop-page-progress-first):

- PROGRESS, a line for each line of the game's scoreboard, in its order:
  its name; its state, LOCKED with the day (an approved look over all the
  line's clip, `locked` on `GET /api/scoreboard/GAME`), READY FOR YOU
  while its confirm row (`confirm_row`) is asking the person (a click
  outlines it in YOUR TURN), else the background and the sprites as two
  bars with the frames matched of the clip's; and one sentence of what is
  being done for it now, counted from the open rows whose target names the
  line (`target_line` on a row of the quest log): fixes being worked on,
  waiting for the person's YES, landing, landed and being measured,
  waiting for an agent (a row up next), asking the person to look, or
  parked (a row in line, which counts as worked only when marked agent
  working). A bar that moved since the page was opened says by how much.
  The page asks for the scoreboard first, with the game from its address,
  before the player, the ROM and the session load; until the first answer
  PROGRESS says "Reading the game's progress." (and a part's sentence
  "Reading what is being done for it." until the quest log comes), and a
  game with no scoreboard says so in one line only once an answer came
  with no lines.
- YOUR TURN, the screen page's, under it: each card still asking, with its
  clip and its box for words open (no WORDS to press), and each row's
  question with its clip and its ANSWER box; NOTHING NEEDS YOU when there
  is none.
- WHAT THE AGENTS ARE DOING, one heading, folded until clicked (kept in the
  browser), with the counts on it: rows in work, rows waiting, rows done
  today (`done_ms` on the scoreboard's endpoint). Open, it holds CARDS YOU
  ANSWERED first (a yes waiting its turn, the one running, and one that
  landed, passed or failed in the last minute, each saying what it is, not
  asking), then the quest
  log with the NEW ITEM box at its top (Enter makes a new item, SEND
  comments on the open row), the steps, the workers and the runner's
  switches, and the clips, drawn as before; while the pointer is on it or
  a box in it is being typed in, it isn't drawn again or reordered, and it
  catches up once the person moves away.

The loop's page shows the screens only while a clip is open on them. Finished rows are folded under DONE with
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
each with the button that does it: the agent's yes cards still asking (YES
and NO; once answered a card is under WHAT THE AGENTS ARE DOING), the
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
the person's NO, or a card that ran out unanswered, sends it to next too,
with their words. A row an agent works on waits; a done row moves only
when its note still talks of the card; only a row's newest card moves it,
once (`row_settled` on the card).

A gate card whose change is on main already (its gate command was run by
hand, say) has nothing to land. Once a minute the runner's
`settle_cards_on_main` checks each open model gate and tool gate card: its
patch is on main when it comes off main's tree (`git apply --reverse
--check` in an index of its own) and doesn't go on again, or, with its
patch gone, when a commit on main has its message's first line. Such a
card closes as landed (`ran.by` is `main`, `landed` the commit), leaves
the page, and its row follows it ("Its card closed with nothing to land:
its change is on main already, as COMMIT"). An answer to such a card does
the same at once, whatever the answer, and the reply's `on_main` says why;
the page says "Nothing to land".

An answer the server doesn't take (it refused it, or didn't answer while
it restarted) stays written on the card in red, "Your answer didn't save:
WHY. The card is as it was.", until an answer to it saves. Before, the
words went into the card and the redraw right after wiped them.

A row asking the person about a card, with no clip, asks nothing of its
own: YOUR TURN shows its card while the card is there and never an open
question once it is gone. `gp mechanic queue` under a loop step refuses
such an ask unless the row's newest card is still in play (open, holding
the person's yes, or running on it): with no card it says to raise the
card first, with a card that already landed to set the row done. Once a
minute the runner's `settle_card_asks` moves each row still asking about a
card that is gone as `settle_card_rows` would, even when the row was set
after the card ended (a card that landed while its worker still ran);
one with no card on file for ten minutes goes to next. On 2026-11-23 a
trouble row whose fix had landed asked the owner "What should happen with
it?".

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

`--look` also takes a theme's folder (its package folder holding
`theme/look.txt`, or the folder holding `look.txt`, such as
`tapper/themes/last-call` or `~/my-theme`): the row's clip keeps the id
the screen offers it by (the folder's name, the package folder's for a
`theme/` folder) as `look` and the folder as `look_dir`, and gp serve
offers that folder under that id for as long as the row names it, ahead of
a copy or a game's theme of the same id. A theme row then has LOOK AT IT
like a model row, with no server of its own: on 2026-11-23 a tapper theme
was shown to the owner through `gp theme look play --background` on a port
of its own ("you shouldn't need to be giving me a separate preview server
like that, the web page should handle this"). The server finds every theme
folder again on each request (`rebuild/themes/index.json` and its files,
`api/screen`'s file times), so an edit shows on the next load, and the
screen adds a theme the server offers since it opened to PICTURES, opening
the model's page again when one is picked that it didn't list.

A model gate's card plays its own change. Its LOOK AT IT, and the LOOK AT
IT of a row that borrows the card's clip, set the clip up beside the model
as the card's patch would leave it, not main's model, so the YES or NO is
on what the yes would land. The server builds that model from the ask
(`$GP_HOME/work/asks/ID.json`, its `--patch`): it copies the game's folder,
applies the patch to the copy, builds the copy's module and lifts its
pictures, and builds the preview into
`$GP_HOME/work/mechanic-preview/GAME/asks/ID/` (crate::mechanic::ask_look).
The page loads it from `rebuild-ask/ID/` and starts it with `ask` on
`POST /api/rebuild/start`. The model's screen then says AFTER THIS CHANGE,
and the clip bar's BEFORE switches to main's model at the same clip (BEFORE
THIS CHANGE), then AFTER switches back. gp serve builds the looks of the
game's open gate cards that have a clip in the background, one at a time,
every half minute once main's preview is built, so LOOK AT IT seldom waits
for a build. A look is built once, from the patch applied to main as it was
then, and is removed when its card is no longer open. A patch that no
longer applies, or that changes files outside the game's folder, gets no
look: the model's screen says why and BEFORE still shows main's. A row's
own clip, live play, a clip the agent shows and the work followed all play
on main's model.

A row asking about a look in the game's own pictures (`look` game) also
has CONFIRM: IT LOOKS RIGHT in the quest log and in YOUR TURN. Such a row
never had a card, so YOUR TURN asks in its own words: its `plain`, else
"Watch PART beside the original. If it looks right, press CONFIRM; if not,
say what is wrong.", PART the scoreboard line it came from, with LOOK AT
IT, CONFIRM and its answer box (a confirm row the scoreboard adds has that
sentence as its `plain`). CONFIRM
(`POST /api/confirm` with the row), or a plain yes typed as the answer
("yes", "looks right", "lock it"), is the owner's yes: the row goes in line
with `confirmed`, and gp serve's runner locks it with no worker, running
`gp mechanic approve` on a worktree of the games tip and landing its
approved.toml through `gp model gate`. The row is then done with the frames
it locked (`locked`). A clip not in the corpus yet waits for the clip
keeping and is tried again every two minutes; any other failure sends the
row to next with why.

However a row's look gets into approved.toml (the runner's lock, a worker's
model gate card, any landing), the runner closes the row within seconds:
an open row, not one a worker is at, whose clip and frames a look saved on
its own name holds goes done with a note `Saved as right: CLIP frames A-B
(N frames, saved DAY)` and `locked`, its clip and `confirmed` gone, so the
page offers no CONFIRM on it again (a row with a target waits in line for
its measure, as a lock's does). A yes on a look already saved locks nothing:
the row goes done saying `Already saved as right, so your yes changes
nothing`. The scoreboard asks for no new `confirm-LINE` row on a line whose
clip has a look saved over all its frames, so a confirmed row that left for
the archive doesn't come back asking, and none on a test line
(`confirm = false` in scoreboard.toml: a clip the loop made, docs/loop.md).

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
| `GET /api/screen/sprite-runs?id=ID` | A corpus clip's sprite runs (SPRITE RUNS): `scanning` while `gp picture --sprites` scans it off the page's thread, then each run's frames, sprites and missing boxes, kept by the clip and the model's version |
| `GET /api/paths/SHA` | A stored path's log, for PLAY FROM HERE |
| `POST /api/confirm` | `{mechanic}`: CONFIRM, the owner's yes to a row's look in the game's own pictures, which gp serve's runner locks |

## The controls' timing

The screen keeps every change of the keys and the gamepad with the time it happened, and each game frame reads the buttons at its own time. Before, the page read the controls once for each game frame it stepped, at whatever moment the browser ran the step. A late browser frame steps up to four game frames at once, and all four read the same buttons, so a short press counted as four frames or a release was seen four frames late. A press and release between two steps was never seen. The gamepad, which a page can only poll, was read no oftener than the screen was drawn.

Since 2026-11-25 the screen page reads them through the same code as the public page: `controls.js` on `gp_controls.wasm` ([docs/controls.md](controls.md)), which the page's server serves beside the screen (`scripts/web/build-play.sh` builds the module beside the player). The older mechanic page did too, with the settings this box keeps, until it was removed. A gamepad on the NES layout is the shell's table, as on the public page: A on the bottom button, B on the left and the right; before, the screen had A on the bottom and the right and B on the left and the top.

A game still reads its controls once a frame, as the machine did: at 30 frames a second a pull of 150 ms is 4 frames or 5 by where it starts. That is the original's own timing and stays.

CONTROLS' TIMING under the screens has these settings, kept in the browser, since controllers differ (GAME SPEED, how fast the game runs as you play it, is the screen's own; the others go to the module, which keeps each inside its range):

| Setting | What it does | Default |
|---|---|---|
| HOLD ASSIST | How long a press is held on after you let go. A pull that fell a little short of what the game asks then counts. 0 is the original's timing | 0 ms |
| READ AHEAD | How far past a game frame's own time its reading may look, never past now. More answers sooner | 8 ms |
| GAMEPAD READ EVERY | How often the gamepad is polled | 4 ms |

The box says how the last press was read: the ms it was held and the frames that read it, which the module counts (`gp_controls_last_ms`, `gp_controls_last_frames`). `?assist=`, `?lead=`, `?poll=`, `?stick=` and `?speed=` in the page's address set them for one visit. `cargo test -p gp-controls` checks the rules.

Not done yet: the editor's free play (`editor/src/FreePlay.tsx`, `Scenarios.tsx`) and the public page's player (`scripts/web/page/preview.js`) still read the controls once a step.
