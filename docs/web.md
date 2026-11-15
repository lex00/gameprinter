# The page and its checks

The public page plays a visitor's own Donkey Kong ROM, and any game its source offers by name (an arcade game from the visitor's romset zip). It is built from the game's package in the games repo beside this one (`../games/donkey-kong`) and holds no game data: the player and the ROM lift are the page's, the game's files and module come from a registry, and the ROM is lifted in the visitor's browser. This file covers the page's checks, the release check, the deploy and the web build's parity with the native interpreter. Modules in the page are in [modules.md](modules.md), registries in [registry-backends.md](registry-backends.md).

## The public page

```sh
just page                    # build into target/page
just web-page                # build into $GP_HOME/work/page
just web-page check          # build, then the headless check and the lift's tests
just web-page accept-pictures  # the picture sections alone, new hashes into pictures.txt
```

`scripts/web/build-page.sh GAME_DIR OUT` writes the page into OUT and the game's play files (the lift's folder and the game's module, published as the package's `play/`) into `OUT.play`. Its last step is `gp template served` on both folders against each ROM the game names. It exits 1 when a file holds the ROM's bytes, lifted files or pictures made from them, and 2 when it can't build or a ROM isn't in `$GP_ROM_DIR` to compare with.

### One page, any game

The page plays any game package its source offers, named in the address. The page's own folder holds the players (the code that runs a kind of game) and nothing of any game; the game, its ROM facts and its themes come from the package, installed and checked by gp-registry.wasm as before.

| Address | Plays |
|---|---|
| `/` | The page's own game (`page.json`'s `game`), in its own theme (`content`): what the page has always done |
| `/p?g=NS/NAME` | That game from the source, latest version; `g=NS/NAME@1.2.3` names a version |
| `&c=NS/THEME` | Opens in that theme package; it must be on the source's by-game list for the game |
| `&look=original` | Opens in the original's look, whatever theme the page would pick |
| `&mode=NAME` | Starts in that mode: `retro`, or a preset of the game's `profiles.toml` (`normal`) |
| `&on=A,B` | Starts with exactly these switches of the game's `profiles.toml` on (a custom set); wins over `mode` |

Every value read from the address is untrusted. A name must look like `ns/name` (lower-case letters, digits and `-`), a version `1.2.3`, a mode or switch a word of letters, digits and `_`; anything else, and any key not in the table, is ignored with no request made for it. A game name must be a `game` package in the source's catalog, a theme must be on its by-game list, and a mode or switch must be one the game's `profiles.toml` has; the page says in words what it left out. No text from the address goes into the page as HTML. The Share button (`share.js`) writes a link of the same shape from what is playing: the game, the theme or `look=original`, and the mode or switches. It never carries the ROM, a save, or anything of the player.

#### What the page reads from the game's package

`play/page.json`, written by `build-page.sh` into `OUT.play/page.json` and published with the rest of `play/`:

```json
{
  "format": 1,
  "title": "Donkey Kong",
  "player": "nes",
  "rom": {
    "kind": "nes",
    "game": "donkey-kong",
    "name": "Donkey Kong: Original Edition",
    "file": "an .nes file of any dump: Donkey Kong (JU), ...",
    "known": ["Donkey Kong (JU)", "Donkey Kong: Original Edition (USA, Europe (3DS Virtual Console))"],
    "known_sha256": { "Donkey Kong (JU)": "..." }
  },
  "howto": ["Controls: arrows move, X jumps, Enter starts. A controller works too."],
  "trademark": "This page is not affiliated with or endorsed by Nintendo. ..."
}
```

`title` names the page while the game plays; `rom.game` is the key the lift keeps the visitor's files under in IndexedDB; `rom.name`, `rom.file` and `rom.known` go in the line under "Load your own ROM"; `howto` and `trademark` replace the bundle's how-to-play panel and About line, as plain text. A package published before `play/page.json` existed (lex00/donkey-kong@1.0.9) is read from `play/lift/game.json` and `play/lift/sources-nes.toml` instead, as the NES player with generic help text. The page's own game keeps its facts in the bundle's `page.json`, so `/` reads nothing more than before.

#### A game with its own pictures

A new game (a copy of a template such as Moss Hop, whose `template.toml` says `source.how = "build"` or `"authored"`) has no ROM: its pictures are its own. `build-page.sh` builds one whole, from its folder's `assets/` or `$GP_HOME/work/assets/SOURCE`, and puts everything the frame reads in `OUT.play`: `bundle.json`, `preview.json`, `assets/`, `profiles.toml` and `profiles/`, `themes/`, `runs/`, and `module.wasm` for a game with hooks. There is no `lift/`. Its `play/page.json`:

```json
{
  "format": 1,
  "title": "Own Hop",
  "player": "own",
  "rom": { "kind": "none", "game": "own-hop" },
  "howto": [],
  "made_by": "the page check",
  "about": "Own Hop is made up for the page check: a copy of the Moss Hop template, with its pictures and no ROM."
}
```

`about` and `made_by` come from the package's `[page]` table, and `howto` too when it has one; with none the page writes the NES keys and the settings line. The page installs such a game by its name, checks every file as for any game, and opens the frame at once with no `files=` (preview.js then reads the installed package's `play/` files): no picker, nothing lifted, no Unload ROM. The line under the title says `Playing Own Hop, made by the page check: it plays from its own files, with no ROM`; About says what it is and who made it. A file dropped on the page is answered `Own Hop plays from its own files: there is no ROM to load.` A theme the source lists for such a game is left out in words, since a theme from the source reaches the frame only through a ROM's lift; the looks in its own `play/themes/` are offered. A finished large-art look in the game folder's `themes/` (a folder with `look.txt` and no `theme.toml`, as `gp game share` puts each theme the person made for the game) goes into `play/themes/` as it is, by gp-web-prep, and is listed in `play/themes/index.json` with the template's own themes; `gp template served` reads it there with the rest of `OUT.play`, and `gp registry publish` with the rest of the package. A link with `look=ID` opens the game in it.

A page from before this kind refuses such a package in words: every page since the one that plays any game by name reads `play/page.json` first and says `Own Hop couldn't be installed: its package has no lift/index.json.` The page on `main` at the time of writing (cc6bd49) predates that: it installs any package named in the address without reading its facts, and goes on asking for a Donkey Kong ROM.

`gp registry publish` refuses such a package if it holds anything of a ROM ([packages.md](packages.md#what-a-package-may-hold)), and `gp game share` publishes a new game this way into the person's own registry (docs/mechanic-page.md).

#### Players

`player` names the code in the page that runs the game, and `rom.kind` what file it takes. `page.js` holds the kinds in `KINDS` and the players in `PLAYERS`, and picks what the picker takes and where the lift reads the file by the kind:

| rom.kind | The picker takes | The lift reads it at | Its lines call it |
|---|---|---|---|
| `nes` | any file (`accept` unset, as the NES picker always was) | `/rom.nes` | a ROM, a ROM file |
| `zip` | `.zip` | `/rom.zip` | a romset, a romset zip |
| `none` (player `own`) | Nothing: no picker | No lift: the frame reads the package's `play/` | its title, what it is and who made it |

| player | rom.kind | Notes |
|---|---|---|
| `nes` | `nes` | The name lex00/donkey-kong@1.0.9 and every NES package give |
| `arcade` | `zip` | Plays silent: the page adds the line "Sound: This game plays silent for now: the page doesn't play an arcade board's sound yet." to the game's how-to-play lines, and silence is no error |

Every kind is lifted by the same `game/lift.js` `liftRom` and `game/lift/gp-web-lift.wasm` (`gp_web_lift::lift_game` knows a zip by its bytes and hands it to `arcade.rs`) and played in the same frame, `game/index.html` with `files=idb:GAME`, which keys an arcade model as a cabinet and draws its board art with `game/gp_board.wasm` (both written by build-preview.sh). The frame fetches `gp_board.wasm` (2.4 MB) only for a model whose `preview.json` names `board`: an NES game's page never asks for it, which every section of the page check except `arcade-*` checks. A package naming a player the page doesn't have, or a `rom.kind` that player doesn't take, is refused in words before any ROM is asked for.

`liftRom`'s contract (`game/lift.js`): it takes the file, the package's `play/lift/` files and the themes, runs the lift module under WASI with the file at the kind's path, and resolves to the lift's `result.json` (`{ ok, sha256, files, found, looks_skipped, looks_partial }` or `{ ok: false, error }`; docs/theme-publishing.md, "In the game", says what the page shows of the two lists), with the lifted files stored under `rom.game/` in the `gp-public` database. The page shows a refusal's `error` as it stands.

build-page.sh writes `player` and `rom.kind` by the template's `source.platform`: `nes` and `nes`, or `arcade` and `zip` (`GP_PAGE_PLAYER` and `GP_PAGE_ROM_KIND` override them).

#### Arcade games: what is built

`/p?g=lex00/tapper` plays Tapper from the visitor's own romset zip once a Tapper package with board art is in the page's source (it isn't yet: the board art is on the games branch `tapper-2`, and publishing it is the owner's call). `scripts/web/build-page.sh GAMES/tapper OUT` builds it as it builds Donkey Kong: the template's platform picks the player and the kind, the game's `roms.toml` goes in the lift's folder as `sources.toml` (build-lift.sh; an NES game's stays `sources-nes.toml`), the ROM line reads "a MAME romset zip of tapper, rbtapper, tapperg, sutapper or tappera, known by its chips whatever the files in it are named" (the sets are the ones the built lift takes, asked of it by refusing a file that isn't a zip; a set it refuses as not drawn right yet, "A set's tables" below, is named after them, as in "a MAME romset zip of tapper or tapperg, ... (rbtapper, sutapper and tappera are refused: not drawn right yet)"), the how-to-play lines give the cabinet's keys, and the About line names no maker. `gp template served` compares both folders against each set's chips (all five, 2026-11-14: no game data). What the page does:

- Before a zip: the ROM line "Load your own Tapper romset (a MAME romset zip of tapper, rbtapper, tapperg, sutapper or tappera, ...) to play. It is checked and read in this browser and never uploaded."; the picker takes `.zip`; the how-to-play lines end "Sound: This game plays silent for now: the page doesn't play an arcade board's sound yet."
- The lift. `gp-web-lift.wasm` reads the zip in memory (gp-arcade's reader, flate2's pure-Rust inflate, no file or network), knows the set by its chips' SHA-1s whatever the files are named, takes the folder whose template accepts the set's identity (a template's `source.accepts`, or its `[[loads]]` entry for a set loaded only to play), lifts with the arcade map and writes the preview's files as for an NES game. On tapper.zip the page then says "Tapper (Budweiser, 1/27/84): all 56 of its assets found. Read in 98 ms, kept in this browser. Playing as Tapper (Budweiser, 1/27/84)." The lift of `tapper.zip` with Tapper's map is the same, byte for byte, as `gp assets lift`'s: both call `gp_web_lift::arcade::lift_files`. The zip support and the board art's file list add 143,155 bytes to the lift after wasm-opt (3,733,285 to 3,876,440), so it stays one module.
- The refusals, each shown on the page as the lift words it, with no game started: "This file isn't a zip. This page takes a MAME romset zip of Tapper: tapper, rbtapper, tapperg, sutapper or tappera."; "This zip holds no chip of a Tapper romset. ..."; "This zip can't be read (...). ..."; "This zip isn't a whole Tapper romset: it has 17 of the 18 chips of tapper (Tapper (Budweiser, 1/27/84)) and lacks tapper_c.p.u._bg_0_5f_12-7-83.5f. ..."; and a set no template takes, "This is NAME (TITLE), a set this page doesn't take. ...".
- The player. `preview.json` names the model's input `device` and, for a model with board art (docs/formats/board-art.md), the lifted files it reads, copied into `board/`. For an arcade model whose state mapping says `board_units` (docs/formats/state-mapping.md, "An arcade original") `board_unit` is the board's screen pixels to a unit (2 on MCR): the page draws the scene's world (not its HUD) that many pixels to a unit and tells gp_board.wasm (`gp_board_units`) to read the positions so; without it, `null`, everything draws as before. `preview.js` keys an arcade model as MAME does: 5 (or the NES layout's Select keys, right Shift and Tab) drops a coin, 1 (or Enter) starts, the arrows or WASD are the stick and X or Space the button; on a gamepad Select is the coin, Start starts and A is the button. The shell's keys on 1 and 5 (a slot's load, a mute) give way to the game, and the key hint under the game, made from the layout in use, says so: "5 coin, 1 start, Shift+1-4 save, 2-4 load, ..., 6-9 mute a channel". The frame rate is the model's (Tapper 30), as for every game.
- The drawing. A model with board art is drawn by the page's own `gp_board.wasm` (`crates/gp-web-board`, `gp_look::board::Companion`): each frame the page takes the game module's canonical state (`gp_state`) and the bodies its scene shows (`gp_scene`), and the board's own drawing (`gp_arcade::video::draw`) makes the 512 by 480 picture from the lifted tiles, sprites, screen and colours. The game's module is unchanged: putting the drawing in it (gp-web) would add crates to every game package's `Cargo.lock`, which `gp game build` builds `--locked`. `gp_board.wasm` was 2,460,923 bytes after wasm-opt before themes (2026-11-14).
- A set's tables. The arcade map reads a table at its `offset` unless `[asset.at]` gives the set's own place, so a set whose program keeps a table elsewhere and has no `at` for it reads other bytes without an error: an earlier copy of Tapper's package drew `rbtapper.zip` with a garbled top of the screen and wrong colours, before the map recorded rbtapper's places. The lift refuses such a set by name (the owner's decision: refused in words, never offered and never hidden): a set whose main CPU chips differ from the map's own set's is refused when any table the map reads from the program (`maincpu`) has no place recorded for that set, and the refusal names them (`gp_web_lift::arcade::tables_unplaced`). A table the set keeps at the same place is recorded with its own offset as the set's place (`rbtapper = 0x0977` beside `offset = 0x0977`), so a place never looked at isn't taken for one that was. The page then says "This is rbtapper (Tapper (Root Beer)), which this page doesn't draw right yet: the places of its program's tables bar_ends, arrivals, slide_objects and 14 more aren't recorded, so its screen, colours or poses would come out wrong. This page takes a MAME romset zip of Tapper: tapper or tapperg (rbtapper, sutapper and tappera aren't drawn right yet).", and every other refusal names them apart the same way. A set with the same program chips (tapperg is tapper's program with other sprites) reads the map's places and is taken.

  Where Tapper stands on `tapper-2` (7ded27f, 2026-11-14): its map records the places of the tables that moved in rbtapper and tappera, and with them each of the five sets drew its own sign and colours on the first bar in the page (later bars not looked at), so the coarse rule before this one (refuse only a set with no place recorded at all) refused none of them. The rule above refuses rbtapper (17 tables with no place for it), tappera (37) and sutapper (52) there. Most of those tables are where tapper keeps them, but not all: lifted from the owner's romsets and compared with tapper's at the same offsets, rbtapper's six bartender tables with no place (`bartender_land`, `bartender_walk_left`, `bartender_walk_right`, `bartender_wait`, `bartender_switch`, `bartender_idle`, added in 7ded27f) and tappera's nine hold other bytes there, so those two sets read the bartender's poses from the wrong bytes; sutapper's eleven screens hold other bytes at the same offsets, which may be its own signs. Each set is taken again once the map records every place, the same ones with their own offset.
- Sound. An arcade model has none unless it binds some: its hooks give no NES APU writes, and the board's own sound (its sound CPU and AY chips) isn't driven by the model yet. The page says so in one line and plays on.
- Themes. A large-art theme draws over the board's pictures by name (docs/formats/board-art.md, "Themes"): a screen by its slot, a sprite as `sprite-ENTITY-CODE`, a counter's digits and tile. preview.js hands a look's rules to `gp_board.wasm` (`gp_board_load_art`) instead of the game's module when the board's art is open, and each frame `gp_board_art_frame` gives the board's picture with what the theme draws left out and the theme's draws, in `gp_art_frame`'s shape with the picture as the draw `["board"]`; the page paints them as any large-art look's, then the theme's particles and overlay over the board's picture in the game's units (two picture pixels each). A theme that names nothing the frame shows draws the board's picture pen for pen. The arcade lift offers a finished theme installed beside the game as the NES lift does (`add_ready_looks`), read against the board's names. `gp theme look play` takes an arcade game. `gp_board.wasm` with the painter is 2,547,741 bytes after wasm-opt, 86,818 more (2026-11-14).

### Checks

| Check | What it does | Needs |
|---|---|---|
| `scripts/web/public/check-page.mjs` | The page in Chrome Headless Shell under its own `_headers`, in sections, each in its own browser context and server: with no ROM nothing plays and the page says so; a wrong ROM is refused naming the right one; each ROM the page knows plays through every stage in the real game's order, in the theme and in the original's look, with its saves, deaths and unload; theme picks that overlap (one made while another's pictures are decoded, a restart in that wait, an older pick finishing after a newer one) end in the newest pick with no failure said, the check holding the frame's picture decoding until it lets each batch go (`theme-picks-*`); the visitor's input on Chrome and Safari rules; the game's module in a Worker and its limits; the install from the registry, and each way a changed source or a changed local copy is refused. Every request each frame makes must be a GET with no body to the page's own server, its source or a `blob:` URL, and no WebSocket may open | A built page; the page's source (the registry it names, on the network, or `--registry-folder`) |
| `check-page.mjs`'s picture sections | The game canvas at fixed moments against the hashes in `scripts/web/public/pictures.txt`; `--accept-pictures` records new ones | A ROM |
| `scripts/web/public/lift.test.mjs` | The lift under Node (`node --test`): a ROM of no Donkey Kong refused naming the ones the page knows, a file that isn't a ROM refused, the JU ROM lifted onto Original Edition's model without 50m and the stage-clear scene, Original Edition with every asset found, a JU dump with its sprite tiles gone refused, a made-up 1 MB ROM searched and refused in under a second, and no theme in the templates. With an arcade game's lift folder, made-up zips: a file that isn't a zip refused naming the game and its sets, a zip of made-up files refused as holding no chip of it, and a zip that can't be read refused (a set is known by its chips' real SHA-1s, so a made-up zip can show only the refusals; the rest is in check-page's `arcade-*` sections and gp-web-lift's own tests) | `GP_WEB_LIFT_OUT`, the lift's module and folder together; the ROM tests skip without the ROMs. `GP_WEB_LIFT_ARCADE_OUT`, the same for an arcade game (Tapper's), else its tests skip |
| `scripts/web/public/share.test.mjs` | The address and the share link under Node (`node --test`): names, versions, modes and switches held to their shape, unknown and repeated keys left out by name only, settings held to a profiles.toml's own names, a share link read back the same with nothing but `g`, `c` or `look`, `mode` or `on` in it, and a game's page facts read from its `play/page.json` or, for an older package, its lift's files | Nothing |
| `scripts/web/test-registry.sh FROM OUT` | Not a check: a copy of a folder registry with a made-up second game (`test/made-up`, the NES player, lift templates named made-up-a and made-up-b) and its theme (`test/made-up-look`), unsigned, for the `game-*` and `share-link` sections. When FROM lists lex00/tapper it also adds `test/tapper-unplaced`, a copy of that package's `play/` with every place its maps record for rbtapper taken out, for `arcade-unplaced`. The release check makes one from its snapshot | target/debug/gp with `gp registry grant` |

The sections about a game named in the address: `game-by-name` (the page's own game by its registry name, then the made-up game with its title, ROM line, how-to-play and About lines from its package, as text; a made-up ROM refused naming it; its own theme by the link, and another game's theme left out in words), `game-bad-name` (a game the source hasn't, a theme named as the game, misshapen names that ask the source for nothing, and settings the game hasn't, each in words), `game-tampered-blob` and `game-tampered-manifest` (the made-up game's play/page.json bytes or its manifest changed on the way: nothing installed), and `share-link` (the link made with no ROM and while playing, read back, and opened in a fresh browser, where it asks for the ROM and then starts in the shared mode and theme). The made-up game's sections are left out, with a `skip` line, when `--registry-folder` doesn't hold it.

The sections about an arcade game, each in its own browser context and run on the page built for Donkey Kong, with the game named in the address (`--arcade-game`, default lex00/tapper) and its zips read from `$GP_ROM_DIR/arcade` where they lie: `arcade-play` (the game installed by its name with its romset line, a picker that takes a zip, the cabinet's keys and the one line that it plays silent; tapper.zip lifted and named; the game playing in the frame with the board's art, the canvas 512 by 480; the key hint under the game the cabinet's (`5 coin, 1 start`, slots loaded on 2-4 and channels muted on 6-9, since preview.js makes the hint from the layout in use and the shell gives way to the game's keys), and the line shown; a coin and a start taken with no error; gp_board.wasm fetched only once a game plays; no sideways scroll at 360 pixels wide; shots `arcade-play-1400.png` and `arcade-play-360.png`), `arcade-refused` (a file that isn't a zip, a zip of made-up files, and tapper.zip with its first chip left out, made in memory and handed to the picker as bytes, each refused in the lift's words with nothing drawn) and `arcade-unplaced` (rbtapper.zip on `test/tapper-unplaced`, refused naming the set and saying it isn't drawn right yet). Each is left out with a `skip` line when the registry folder hasn't the game (`arcade-play`, `arcade-refused`) or `test/tapper-unplaced` (`arcade-unplaced`), or the zip it plays isn't in `$GP_ROM_DIR`. Every other section checks that no frame asked for gp_board.wasm. The arcade game has no picture moments in `pictures.txt`: a moment's hash, like the NES ones, is text and holds no picture, but the moments are played by the NES routes (a ROM's `ROUTES` entry, by its headerless SHA-256), and Tapper's package isn't published yet and its art still changes on its branch, so references recorded now would go stale before they could catch anything.

The section `theme-hud-over` plays Moss Hop, the project's own game, in the page's own player (game/index.html, preview.js and gp_web.wasm from the built page), with Moss Hop's files and two made-up large-art themes served beside them from `DIST.mosshop` (or `--mosshop DIR`), which `scripts/web/mosshop-hud.sh DIR` writes and the release check makes: a copy of Moss Hop with its HUD's counters in the background, `tall`, whose hero is drawn 200 pixels tall so it stands across the HUD's row, and `tallover`, the same with `#+ hud_over` (docs/formats/theme.md, "Large-art rules"). Held 60 frames in, the two frames differ only in the box of the HUD's text: hero-coloured in `tall`, the text in `tallover`. Without the folder it is skipped with a line saying how to make one. In the release check a missing Moss Hop ROM (the script's exit 3) is a noted skip, and a run of the script that fails fails the release check. Run against a page built before the line, every check passes but the first difference: the older page offers `tallover` and draws it as `tall`.

The section `theme-in-region` plays the same build in a third made-up theme, `instart`: the hero 200 pixels tall in one colour, and in Moss Hop's one region of the stage, `start` (its level kit, docs/level-kit.md "Regions of a stage"), 16 pixels tall in another (`#+ in start pic ...`). It checks that the build's `preview.json` gives the region as `["start", "level-main.png", 0, 160, 64, 32]` and that the module took it, that standing at the start only the short picture's colour is drawn, and that after walking right out of the region and standing again only the tall one's is. A page from before regions draws the tall picture in both.

The section `theme-sound-takes` plays Moss Hop in the page's own player the same way, from `DIST.mosshop-takes` (or `--mosshop-takes DIR`), which `scripts/web/mosshop-takes.sh DIR` writes and the release check makes: a copy of Moss Hop with a counter `beat` that counts 0 to 39 and wraps every frame, one made-up sound role `tick` cued while it is 0, and a made-up large-art theme `takes` that gives the role three takes (`sound/roles/tick.wav`, `sound/takes/tick.2.wav` and `tick.3.wav`, square tones at three pitches made by the script). A run of 400 frames in the theme: the page reads the three files, the module's log (`gp_sound_role_log`) has the role start 9 times, 40 frames apart, playing takes 1 2 3 1 2 3 1 2 3, and the same run played again in a fresh load starts the same takes at the same frames. Run against a page built before takes (2026-11-14), the theme loads with no error and plays, but the page reads only `sound/roles/tick.wav` (4 fewer requests over the two runs), so the five checks on the takes fail and the rest pass.

The section `arcade-theme` does the same for an arcade game: Tapper's preview from the player's own `$GP_ROM_DIR/arcade/tapper.zip`, with its module and one made-up theme, `flat` (the bartender's sprite at every code drawn as a flat square, nothing else named), served from `DIST.tapper-theme` (or `--tapper-theme DIR`), which `scripts/web/tapper-theme.sh DIR [GAME]` writes (the lift goes to `$GP_HOME/work/tapper-theme/assets`, never into a repo). The same 700 frames from a coin and the start are drawn in the board's own look and in the theme: the theme's square is one 32 by 32 box where the board draws the bartender's sprite (his place in the game, `x - 3` and `241 - y` units of two pixels), no pixel outside it differs, and the page fetched the theme's look, rules and one picture and nothing else of it. Without the folder it is skipped with a line saying how to make one.

Which sections of check-page.mjs need a ROM: the `arcade-*` sections need a zip, as above; `no-rom`, `wrong-rom`, `registry-link`, `registry-links`, `registry-goes-bad`, `registry-local-change`, `game-*` and the `registry-bad-*` sections don't; `share-link` plays its last part only with a ROM. The `rom-*`, `pictures-rom-*`, `visitor-*`, `later-visit`, `kept-look`, `theme-picks-*`, `module`, `registry-rom-*` and `registry-theme-*` sections play a ROM and are left out, with a `skip` line, when it isn't in `$GP_ROM_DIR`. None of them runs in `just check`, which holds only what this repo builds on Moss Hop: every section needs the page, which is built from the Donkey Kong package in the games repo, and installs the game from the page's registry. They run in `just web-page check` and in the release check.

check-page.mjs takes playwright-core from `scripts/web/node_modules` (`npm ci --prefix scripts/web`, which the recipes run when it's missing), as check-mechanic.mjs does.

## Serving a game on this machine

A game played on this computer runs under the public page's own headers, read from `scripts/web/public/_headers` by one function (`gp_mcp::page_headers`, built into gp): the Content-Security-Policy with no inline script or style, and the two Cross-Origin headers the game's module needs for its shared buffer. Every local way of serving a game uses it:

| Server | What it serves |
|---|---|
| `gp registry serve REG --page PAGE` | A built page and a registry; the page's own `_headers`, by path, and the public page's when it has none |
| `gp theme look play` | The page with a registry, or a game's own files (its `_headers` is the public page's) |
| gp serve and the mechanic page | The game's frame: the rebuild's preview, a new game built to play, `/share/` |
| `gp host serve --play DIR` | A build `build_publish` wrote, as a playable link at `/play/ID/index.html?files=here` |

A plain file server sends none of them, and the frame then refuses to start any game, not only one with a module, since even the page's own interpreter runs through the module host: under python's `http.server`, a Moss Hop preview showed `The preview didn't start: this page wasn't served with its isolation headers, so it can't run a game module with its limits` and played no frame, and the same build under `gp host serve --play` played 358 frames in six seconds. So `build_publish` names `gp host serve --port 0 --play OUT`. The preview's style is in `preview.css`, since the policy allows none inline.

## Deploying the public page

The host is Cloudflare Workers static assets: a Worker with no code that serves the page's folder, named in `scripts/web/deploy/wrangler.toml`. Nothing deploys on a push; the deploy runs from this machine, after the release check.

```sh
just web-page-release        # build the page at this tip and run every check, no upload
just web-page-release --full # the same with nothing reused
just web-page-deploy list    # the full release check, then what would upload; stops there
just web-page-deploy         # the full release check, what would upload, a prompt, then wrangler
```

Wrangler is `4.157.0` from npm and needs `npx wrangler@4.157.0 login` once, which opens a browser and keeps the token outside the repo.

`scripts/web/release-check.sh` runs every step, prints each step's time, and ends with PASS or FAIL. It builds into `$GP_HOME/work/page-release/<commit>-games-<games commit>`, each with `-dirty-<digest>` when that repo has uncommitted changes, so a new folder for each state of the two repos.

| Step | Fails when |
|---|---|
| A snapshot of the page's source: a shallow clone of `lex00/gp-registry` at the commit `main` is on, into `<page>.registry`, its commit printed, and a copy of it with the made-up second game (`scripts/web/test-registry.sh`, into `<page>.registry.any`); `check-page.mjs` reads the copy with `--registry-folder`, so a publish during the run changes nothing | The clone fails (no network, say), or the made-up game can't be added. The check never reads the live registry instead |
| `build-page.sh`, ending in `gp template served` | A file holds the ROM's bytes, lifted files, pictures made from them or a game image; or the ROMs aren't in `$GP_ROM_DIR` to compare with |
| `check-page.mjs` under the page's `_headers` | Any check of the page fails, a policy refusal in any frame among them |
| `lift.test.mjs` on the page's lift files, `theme-line.test.mjs` and `share.test.mjs` | Any of the lift's tests fails, or the theme line rule passes a page that offers a theme its source doesn't |
| `gp package scan` on the game's package | It has a needs-rewrite item, or it wasn't compared with the ROMs |
| The size and files | Over 20000 files or 25 MiB in one file (the Workers static assets limits). It prints every file with its SHA-256 and writes them to `<page>.manifest.txt` |

It builds the page with `GP_PAGE_CONTENT=lex00/turnips`, the deployed page's theme, unless `GP_PAGE_CONTENT` is set (empty for none); check-page.mjs's theme checks fail on a page with no theme. On a pass it records the page's digest (`scripts/web/page-digest.py`: one SHA-256 over every file's SHA-256 and path) in `$GP_HOME/work/page-release/passed/DIGEST`. It also fails if the page's digest after the checks isn't the one it had after the build.

`scripts/web/deploy.sh [PAGE_DIR] [--list]` uploads a folder only when its digest has a pass recorded, so what goes up is what passed, file for file: a file changed, added or removed after the check, or a page built from other sources, has another digest and is refused. With no PAGE_DIR it runs the full release check first and takes the page it built, which is what `just web-page-deploy` does. It shows the worker, the folder, the file count and size, the pass it goes up under and the command, asks for the worker's name to be typed, takes the digest again, and runs `npx wrangler@4.157.0 deploy --assets PAGE_DIR`. `--list` runs wrangler's `--dry-run`, which uploads nothing. `GP_WRANGLER` replaces the wrangler command, so the script can be tried up to the upload against a stand-in. Cloudflare keeps earlier versions: `npx wrangler rollback` from `scripts/web/deploy` returns to the last one.

On 2026-11-11, at this branch's tip with uncommitted changes and games at 07d52be, with both ROMs in `$GP_ROM_DIR`, the release check passed in 448 s: the build in 32 s with the modules kept (471 s with nothing kept), 32 page sections in 400 s (one at a time: the machine's load was near 80), the lift's 7 tests in 8 s, the package scan with 0 needs-rewrite items, and a page of 22 files in 7.19 MiB. The run before it, built with no theme, failed 17 checks in `rom-*-play` and `kept-look`. The 1 MB search test took 741 to 1184 ms under that load against its limit of 1000 ms, so it can fail on a loaded machine.

### Reuse

The release check reuses what didn't change: the optimized modules and gp-web-prep's output (build-preview.sh and build-lift.sh with `GP_WEB_REUSE=1`), and a package scan with nothing to rewrite. Each is kept in `$GP_HOME/work/page-release/reuse/` (`$GP_PAGE_REUSE`) by `scripts/web/reuse.py` with the files it was made from or read and their SHA-256s, and is used only when every one of them is unchanged. A reused result is printed with "kept". `--full` makes and checks everything again and keeps the results. The page check runs every section each time: the game comes from a registry, which a kept result's inputs can't name. Deleting the store is safe at any time.

## Parity

```sh
just web-parity
```

`scripts/web/parity.sh OUT MODEL ASSETS CLIPS` checks a preview build (`scripts/web/build-preview.sh OUT MODEL ASSETS`) against the interpreter built for this machine. CLIPS is a TOML list of baselines under `scripts/web/parity/`. For each, gp-web-prep plays the baseline's input natively and writes the frames and the state hash at power-on and after every frame; `scripts/web/parity.mjs` then plays the same frames in the build's `gp_web.wasm` under Node and requires every hash and the spec digest to be the native run's. `scripts/web/parity/mosshop.toml` holds Moss Hop's five baselines, so it needs no ROM but Moss Hop and runs in `just check`. On 2026-11-11 every frame of the five (1,200 frames) agreed. With the module built it takes about 30 s; the first build of gp_web.wasm and gp-web-prep in a fresh target folder took 3 min 40 s.

The native side was `gp interp run` in the old tool. Now `gp interp run` runs a model through its game's module and has no `--baseline`, `--frames`, `--level` or `--json`, and Moss Hop has no module, so the native run is gp-web-prep's. A clip of button spans, a store recording or another level, which the old lists had, is refused until gp can play those natively again. gp-web-prep doesn't write the hooks' label, so the native label is taken as empty, which is right for a model with no hooks in crates/gp-exts, such as Moss Hop.

### An arcade game's parity

```sh
just web-parity-arcade [CLIP...]
```

`scripts/web/parity-arcade.sh OUT GAME_DIR ASSETS MODULE ROM CLIP...` holds an arcade game's model in the page's player to the native run. For each corpus clip, `gp clip log` writes its input from power-on, `gp interp translate` turns the original's state at the clip's start into the model's, `gp interp run --state` plays the clip's own frames from it natively through the game's module, and `parity.mjs --wasm MODULE` plays the same frames from the same state in that module under Node through the page's exports (`gp_load`, `gp_load_state`, `gp_run_frames`). Every state hash and the spec digest must agree, and the native run's hooks label must be the module's BLAKE3. The recipe lifts `tapper.zip` from `$GP_ROM_DIR/arcade`, builds Tapper's module from the kit and runs `tapper.l1.001`, `tapper.die.001` and `tapper.clear.001`; it skips without the games checkout's `tapper/` or the zip. On 2026-11-14 those three and eight more (start, l4, l9, l15, bonus, tip, gameover, score), 9,345 frames, agreed. A clip starts from the original's state because Tapper's model has no boot, attract or credit play: from power-on its bartender stands at 0, 0 and no level starts.
