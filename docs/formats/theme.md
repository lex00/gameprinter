<!-- Brought from game-printer's docs/formats/theme.md with the theme commands (wave 8 of the
port), which read and write this format. Paths under templates/GAME are a game's
folder in the games checkout in gameprinter (games/GAME). -->

# Themes

A theme restyles a game by role without touching its model (ARCHITECTURE.md, "Themes"). This page is the theme format and the stack format, version 0.1.3, how stacks resolve, the readability check, the Style checks, and where a theme is drawn. The Rust types in `crates/gp-theme/src/format.rs` are the definition. `schema/theme.json` and `schema/theme-stack.json` are generated from them (`gp theme schema theme|stack`), and a test fails when either is stale.

A theme is presentation only. It lives in the `present` side of every runtime: the generator's pictures, the web preview's canvas, the Bevy build's render crate, the CPU frames of `gp theme video` and `gp video`. Nothing in the simulation reads it, so no state hash can depend on it, and floats are fine throughout.

## Run it

```sh
gp theme check templates/mosshop                       # the template's own stack: each theme, readability, Style
gp theme check templates/mosshop --theme templates/mosshop/themes/paper
gp theme video templates/mosshop --baseline baselines/mosshop/stomp-then-pit.toml --themes -o /tmp/dusk.png
gp theme paint templates/mosshop -o /tmp/paper --id paper
gp-web-prep --themes templates/mosshop LIFTED OUT BASELINE...
```

| Option | Means |
|---|---|
| `--themes` | The model's own stack: the template's `themes/stack.toml`, then the player's `$GP_HOME/themes/stacks/<model>.toml` |
| `--stack FILE` | A stack file, its baked layers from the template's `themes/` and its personal ones from `$GP_HOME/themes/` |
| `--theme DIR` | A theme directory, as a personal layer on top; repeat for more |
| `--allow-unreadable` | Go on when the readability check refuses |

| Command | Options |
|---|---|
| `gp theme check` | All four; `--themes` when given none |
| `gp theme video` | All four |
| `gp-web-prep` | `--themes`, `--stack`, `--theme` |
| `gp video` | `--theme` only |

## A theme

A theme is a directory with `theme.toml` and the files its restyles name.

```toml
format = "0.1.0"
id = "dusk"
title = "Dusk"
license = "CC0-1.0"

[[palette]]
target = "world"
hue = -20.0
lightness = 0.7
tint = "#3a2a5a"
tint_amount = 0.25

[[effect]]
kind = "gradient"
target = "sky"
colors = ["#1b1440", "#d8604a", "#f4a85a"]

[[effect]]
kind = "particles"
style = "fireflies"
color = "#ffe27a"
count = 24
```

| Key | Meaning |
|---|---|
| `format` | `"0.1.0"`. A reader takes the same major and minor version with a patch no newer than its own |
| `id` | Lowercase letters, digits, `-` and `_`, starting with a letter. A stack names the theme by it, and it's the theme's directory name |
| `title`, `description`, `author` | For people |
| `license` | The theme's own license, an SPDX expression |
| `for` | Optional: the models it was made for, by `model.name`. A stack applying it to another model is refused, and so is a look drawn for one (`gp_look::looks::offered`, the lift). `gp theme check` checks the restyles' slot names against each (below) |
| `palette` | Live effects: palette rules (below) |
| `effect` | Live effects: picture, particle and screen effects (below) |
| `restyle` | Restyled assets (below) |
| `style` | The style guide Style checks measure against (below) |
| `audio` | Restyled audio: sound bank mappings by sound role (below; since 0.1.1) |
| `sound` | Sound replacements: a file for each of the model's sound roles (below; since 0.1.3) |

Unknown keys are refused, as in the model.

### Targets

Rules and effects name a target. It's a role from the model's role table, a role group from `gp_model::validate::ROLE_GROUPS` ([the model](../model.md), "Roles"), or `*` for every pixel. A theme names no sprite, tile or slot of a particular game, except in a restyle of a slot, so a theme made for one template applies to any other with the same roles.

Every pixel the presentation draws carries a role:

| Pixel | Role |
|---|---|
| A sprite's opaque pixel | Its entity's `role`, in every frame: each state's, each animation clip frame's, and each tile of its layout (`cells`) |
| A HUD pixel: its layer and its glyphs | `hud` |
| A level cell's pixel | The metatile's `item_roles` entry; a level built from objects takes its palette's `item_roles` entry instead |
| Color 0 of a level cell (the backdrop) | `sky` |
| A recording's pixel in `gp video` | None: only `*` rules apply there |

Core 0.3.1 requires `item_roles` on every metatiles and palettes slot, so every level pixel has a role.

### Palette rules

```toml
[[palette]]
target = "enemy"
replace = [["#f83800", "#20a040"]]
ramp = ["#000000", "#5a2e6a", "#ffffff"]
hue = 30.0
saturation = 1.2
lightness = 0.9
tint = "#e04a30"
tint_amount = 0.35
```

Each field is optional and they apply in this order. `replace` swaps exact colors; a pixel it matches keeps its new color and skips the rest. `ramp` maps each pixel's CIELAB lightness onto the colors from darkest to lightest, as a gradient map (two or more colors). `hue` adds degrees, `saturation` and `lightness` multiply (0 to 4), and `tint` mixes a color in `tint_amount` of the way (0 to 1, default 0.5).

In one theme, only the most specific rule for a pixel's role applies: one naming the role, else one naming its group, else `*`. Of two rules with the same target, the later one in the file applies.

### Effects

| `kind` | Fields | Does |
|---|---|---|
| `gradient` | `target`, `colors` (two or more) | The target's pixels drawn as a vertical gradient over the picture, top to bottom (a gradient sky) |
| `glow` | `target`, `color`, `radius` (1 to 3, default 1) | An outline of `color` at alpha 160 on transparent pixels within `radius` of the target's pixels |
| `particles` | `target` (an entity role or `screen`, the default), `style` (`fireflies`, `snow`, `rain`, `sparks`), `color`, `count` (1 to 256, per body for a role), `seed` | Particles over the screen, or around each active body of the role |
| `scanlines` | `strength` (0 to 1) | Every other row darkened |
| `vignette` | `strength` (0 to 1) | The corners darkened, with a smooth falloff from half way out |
| `tint` | `color`, `amount` (0 to 1) | The whole screen mixed with `color` |
| `crt` (0.1.2) | `strength` (0 to 1), `mask` (default true) | A television's picture: a dark gap under each line, the corners darkened, and with `mask` the columns tinted red, green and blue in turn (an aperture grille) |
| `grid` (0.1.2) | `strength` (0 to 1) | A handheld's pixel grid: a dark line between pixels |

Gradients and glows are drawn into the pictures. The overlay is one picture made from tint, vignette, scanlines, `crt` and `grid`, drawn over everything. Tint is drawn first and the CRT and the grid last, with the vignette and then the scanlines between them. A CRT or a grid needs more than one overlay pixel to a game pixel, so `gp_theme::Resolved::overlay_fine` draws the overlay at three to one each way and the Bevy build draws it over the view's size; at the view's size (`overlay`, which the web preview and the CPU composer use) a CRT is its gaps on every other line and its corners, and a grid isn't drawn. Particles are a pure function of the frame number, the view and the boxes of the bodies they follow: `crates/gp-theme/src/particles.rs`, which depends only on `core`, so the generator copies it into a generated game's render crate as it is, and `scripts/web/page/preview.js` has the same formulas in JavaScript. Each particle's position comes from a 32-bit integer hash of `(seed, index, salt)`, so every runtime draws the same particles, to within float rounding.

### Restyled assets

```toml
[[restyle]]
target = "player"
file = "player.pix"
origin = "generated"
license = "CC0-1.0"
sha256 = "..."
generator = "gp theme paint --id paper --seed 1"
```

| Fields | Replaces |
|---|---|
| `target` alone (an entity role) | Every sprite of entities with the role, in every state and every animation frame (a sprite-set's frames too), with `file`, standing on the body's bottom edge like any sprite. A restyle by state or by frame is an open question |
| `slot` and `target` (an item role) | Every cell of that role in levels drawn from the metatiles slot `slot`, with `file`, 16 by 16 |
| `slot` alone | The slot's whole file. Only a tiles slot, with an indexed PNG of the slot's size, or (since 0.1.3) a `.pix` of three colors and transparent, whose color lines in order are the console palette's entries 1, 2 and 3 |
| `slot` and `entry` (since 0.1.3) | One entry of a layout slot (update strings: a stage's screen, the HUD's layer, a title) drawn as `file`, a picture the size of what the entry draws, in place of its tiles. Its opaque pixels keep the role the game's tiles gave them; its clear pixels are the backdrop (the HUD's are clear) |

| Field | Meaning |
|---|---|
| `file` | Relative to the theme's directory; absolute paths and `..` are refused. `.pix` (below) or `.png` (RGB or RGBA, 8 bits) |
| `origin` | `generated`, `uploaded` or `drawn` |
| `license` | An SPDX expression, as in `slots.toml` ([stubs](../stubs.md)). `LicenseRef-own-copy` is refused: a theme is shared, and art from someone's own copy of a game can't be |
| `sha256` | The file's SHA-256 in hex. A theme whose file doesn't match is refused |
| `generator`, `author`, `note` | Optional, as in `slots.toml` |

A restyle of a slot the simulation reads (`gp_interp::lifted::simulation_slots`) is refused, as `assets_place` refuses one.

### Slot names

A runtime passes over a restyle whose slot or entry its model lacks, with no word (`gp_look::present::art_themed`). That lets one theme serve two versions of a game, and it hides a typo. `gp theme check` checks each restyle that names a slot against every model in the theme's `for` list, or against the model checked when the theme has no `for` (`gp_theme::slots`, `crates/gp-cli/src/theme_slots.rs`). It reads the other models from the templates beside the checked one whose `template.game` is the same (Donkey Kong JU and Original Edition are both `donkey-kong`). A model's entries of a slot are the `{ slot, entry }` pairs the model names and the entries its asset map lists for the slot's source asset.

| Case | Result |
|---|---|
| No model has the slot | Refused, naming the slot and the nearest real slot name by edit distance: ``REFUSED restyle 1 (sprite_tiles.pix): no slot `sprite_tile` in donkey_kong_oe; the nearest is `sprite_tiles` ``. Nothing is drawn |
| The slot's entries are known and no model has the entry | Refused the same way, with the nearest entry of that slot |
| Only some models have the slot or entry | A note naming which: ``note: restyle 3 (title_tiles.pix): slot `title_tiles` is only in MODEL_A; MODEL_B passes over it`` |
| A model in `for` has no template of the game beside the checked one | A note that it wasn't checked |

The check's JSON has `slots`, with an entry for each layer. The entry lists the models checked and those not found, and each finding (`restyle`, `slot`, `entry`, `refused`, `models`, `nearest`, `message`).

`.pix` is text pixel art, so a theme's small assets review and diff in git:

```text
pix 1
# a comment
size 16 16
. transparent
a #2a221c
b #5ca84e
................
....aaaaaaa.....
```

After the header and size come color lines (one character other than a space, then `transparent` or `#rrggbb`) and then `height` rows of `width` characters. `gp theme paint` writes them.

### The style guide

```toml
[style]
palette = ["#2a221c", "#5ca84e", "#9cd67e"]
max_palette_distance = 4.0
edge_density = [0.1, 0.8]
size_tolerance = 0.35
```

With a theme on, the contract's Style level measures art against the style guide of the highest layer that has one. With none, Style checks measure against the original art, as before.

### Restyled audio

Added in format 0.1.1, a remaster plays the model's notes through other instruments. Each entry names a target in the `sound` role group and a sound bank mapping file (docs/formats/sound-banks.md) in the theme's directory. The notes, and so the simulation, don't change; only the sound does.

```toml
[[audio]]
target = "music"
banks = "audio-banks.toml"
license = "CC0-1.0"
```

| Key | Meaning |
|---|---|
| `target` | `sound` or `*` (every sound slot), `music` or `sfx` (every slot of that kind of role), or a slot role (`music.level`, `sfx.game`) |
| `banks` | The mapping file, relative to the theme's directory; it must parse, and its banks are found in the theme's directory and the bank folders |
| `license` | The mapping's and its banks' license, an SPDX expression; never `LicenseRef-own-copy` |
| `note` | For people |

The highest layer with an entry that applies to a slot's role wins (`gp_theme::Resolved::audio_mapping`). The Bevy generator renders that slot's songs through the mapping (gp-sound-bank); without one they play on the APU. gp-mcp's `sound_edit` writes a theme's mapping when given `theme`, adding the entry if the theme has none (docs/sound.md).

### Sound replacements

Format `0.1.3` added a file for each sound role the model names (core feature `present.sound_roles`, `[[present.sound.roles]]`, docs/model.md), played on the role's cue in place of the model's own sound. While any layer replaces a role, the model's own sound (its bound songs, or a sound driver's APU writes) is silent, so with a file for every role a game's sound data isn't needed.

```toml
[[sound]]
role = "jump"
file = "jump.tune"
origin = "generated"
license = "CC0-1.0"
sha256 = "..."
```

| Key | Meaning |
|---|---|
| `role` | A sound role of the model |
| `file` | `.tune` (text notes, `gp_theme::tune`), `.wav` (16-bit PCM) or `.ogg` (Vorbis), relative to the theme's directory. A template's theme uses `.tune`, since a template holds only text |
| `origin`, `license`, `sha256`, `generator`, `author`, `note` | As in a restyle |

Every runtime plays them with one piece of code, `gp_present::sound::RolePlayer`: after each tick it reads each role's cue from the world (the role's counter test; a held role starts on its first frame and stops when the test fails) and acts on it from the next frame, as a game's driver takes the requests the last frame left, starts the role's samples (a music role replaces the music role playing, an effect starts again from its beginning, a looping role repeats while it plays), and gives the frame's 735 samples at 44,100 Hz, added and clipped to 16 bits. While a test of the model's `present.sound.silent_while` holds (feature `present.sound_silent_while`; Donkey Kong's attract demo, which asks for its tune with the channels off), nothing plays. The generator and `gp-web-prep` render each role's file to `assets/sound/roles/<role>.wav` (16-bit mono, 44,100 Hz); the Bevy build (its `audio` feature, on by default for `play`) and the web preview stream the player's samples as they stream a sound driver's, and while it holds any sound they work out no driver writes and play no bound song, so nothing reads the game's sound data. `gp theme video --wav FILE` plays the same player over a run and lists the frame of each cue.

A `.tune` starts with `tune 1` and `step N` (frames a step lasts, at 60 a second). A tune that repeats while its role holds has `loop`. Then come the voices: `voice NAME WAVE VOLUME` (waves `square`, `pulse`, `triangle`, `soft`, `pluck`, `noise`, `thud`) and `notes` lines of `C4:2`, `-:1` (a rest) or `C4>G4:4` (a glide). It may also hold effects described by parameters, sfxr's way (Tomas Pettersson's generator, MIT, https://drpetter.se/project_sfxr.html; ours is our own code, `gp_theme::sfx`): `sfx NAME VOLUME key=value ...` with `wave` (`square`, `saw`, `sine`, `noise`), `freq` (a note or Hz), `slide` and `dslide` (semitones a second, and its change), `min` (the pitch it stops below), `attack`, `sustain` and `decay` (frames), `punch` (0 to 1), `duty` and `duty_sweep`, `vibrato` (semitones) and `vibrato_rate` (Hz), `arp` (semitones) and `arp_at` (frames), `repeat` (frames), `lowpass` (0 to 1, 1 open), `highpass` (0 to 1, 0 off) and `at` (the step it starts on). A tune holds voices, effects or both: `sfx zap 0.5 wave=square freq=C6 slide=-36 decay=12 duty=0.3`. It renders to the same samples on every machine at 22,050 Hz. The highest layer with a file for a role wins (`gp_theme::Resolved::sound_file`). How a game's kit names its roles and how the paper theme makes its tunes is in [the theme kit](../theme-kit.md).

## Themes found at run time

The settings screen ([the settings screen](../settings.md)) offers the themes a runtime finds beside it, and the one chosen is drawn from the next frame, with no restart: the simulation never reads a theme. What the runtime finds is a look, a theme already drawn for this game: a folder `themes/<id>/` with `look.txt` and the files of the build's `assets/` that the theme changes, under the same names (pictures, `present.txt`, overlays, and `sound/roles/<role>.wav`). A runtime shows a picture from the look's folder when the look holds it, else the build's own. The code is `gp_present::looks` (the manifest) and `gp_gen_bevy::looks` (drawing one).

```text
gp-look 1
id paper
title Paper
own_sound
file frame-0-0.png
file sound/roles/jump.wav
```

| Line | Means |
|---|---|
| `id`, `title` | The theme's id and title |
| `original` | The look of the lifted art and the game's own sound |
| `own_sound` | The game's own sound plays (the look replaces no sound role) |
| `file` | A file the look holds, relative to its folder; never absolute or with `..` |

#### A sound role's takes

A look gives a sound role its sound as `sound/roles/<role>.wav`, a 16-bit PCM WAV listed as a `file` line (a role is named in the model's `[[present.sound.roles]]` and the game's theme kit). Since 2026-11-14 (issue #30) a role can have more takes: `sound/takes/<role>.2.wav`, `<role>.3.wav` and on, each listed as a `file` line too. Each time the role starts the runtime plays its next take, in the order of their numbers, then the first again (`gp_present::sound::RolePlayer`, which counts the role's starts from the first frame, so a run played again from power-on plays the same takes at the same frames). `gp_present::looks::role_take` reads a file's role and take from its path.

```text
gp-look 1
id clay
title Clay
art art.txt
file sound/roles/jump.wav
file sound/takes/jump.2.wav
file sound/takes/jump.3.wav
```

The takes are in a folder of their own because of what a page from before them does: it plays every listed file under `sound/roles/` as the role of the file's name and stops loading the look's sounds at a name the game hasn't got, so `sound/roles/jump.2.wav` there would cost the roles listed after it their sound. It never reads `sound/takes/`, so it plays `sound/roles/jump.wav` every time and the rest of the look as before. A role needs its `sound/roles/` file for that reason: with only takes, a page from before plays nothing for it. No line of the manifest is new, so every reader of `look.txt` takes such a look as it is. `gp theme look check` lists each role's files and refuses the files a page can't play ("Sound" in its report).

| Runtime | Where it looks | What it writes |
|---|---|---|
| Bevy build | `assets/themes/<id>/` (the build's own), `themes/<id>/` beside the executable, and `themes/<id>/` in its data folder (docs/generator.md) | `gp gen bevy` draws each theme in the template's `themes/` as a look, and `original` when the assets were lifted from a ROM (`[source] how = "rom"`) and the stack draws a theme |
| Web preview | `themes/index.json` beside its bundle, which a static host serves: `{"themes": [{"id": ..., "title": ...}]}`, then each `themes/<id>/look.txt` | `gp-web-prep` the same as the generator; the public page's in-browser lift draws the template's themes on the visitor's lifted art |

A runtime also lists the themes published for its game and downloads a chosen one's look into a cache: the same folder layout in the Bevy build's data folder, with `package.txt` naming its package, and the browser's Cache Storage in the web preview ([theme publishing](../theme-publishing.md#in-the-game)).

The build's own look is the one its stack drew (its top layer's id, or `original` with none). The settings list it first, then each look found, `original` only where the assets were lifted. Pictures are swapped by number, so a runtime refuses a look that draws other pictures than the build's (by name and number) when it is drawn. The overlay and particles of a look aren't swapped yet. What a look changes is the pictures, the presentation and the sound.

### One theme for two templates

A template's `themes/<id>/` holds the theme, or only `same_as.toml`, which points at a theme folder of another template beside it:

```toml
theme = "../../../donkey-kong-oe/themes/paper"
```

The path is relative to the pointing folder and must name `TEMPLATE/themes/ID` in the same templates folder. The pointer is followed once (`gp_theme::same_as`): its target must hold `theme.toml`, and the theme's `for` must name the pointing template's model. Everything that reads a template's themes reads the target's files in the pointer's place:

| Reader | What it does with a pointer |
|---|---|
| `gp_theme::stack::load` | Loads the target; the layer's `dir` is the target, so restyle files are found there. A baked stack layer may name a pointer folder |
| `gp_look::looks::offered`, `gp-web-prep` | Offer it as a look like any theme, refused when `for` doesn't name the model |
| gp-web-lift | The same, for the public page's in-browser lift. A pointer that doesn't resolve is listed in `looks_skipped` |
| `scripts/web/build-lift.sh` | Ships the pointer, naming the target by its folder in the bundle, when the target's template is in the bundle; otherwise ships the target's files in its place |
| `gp theme check`, `gp theme publish` | Read the target (`--theme` may name the pointer folder) |
| `gp template check` | Accepts a `themes/paper/` that is a pointer whose target checks and whose `for` names the template's model |
| `gp template pack`, `gp package scan`, the asset-free scan | Read the target's files under the pointer's folder name, so a packed template stands alone |

A missing target is refused with the file and the reason, and so is a target that is itself a pointer (a chain or a loop). An absolute path and a path outside the templates folder are refused the same way. From 2026-11-04 to 2026-11-06 Donkey Kong's paper theme was kept once, in `templates/donkey-kong-ju/themes/paper/`, painted from Original Edition (the model with more slots) with `gp theme paint templates/donkey-kong-oe --for donkey_kong`; JU passes over the two restyles of slots it lacks. `templates/donkey-kong-oe/themes/paper/` held only `same_as.toml`. Since 2026-11-06 it is in `templates/donkey-kong-oe/themes/paper/` and for `donkey_kong_oe` alone, and the public page's lift ships a pointer to it for the Donkey Kong (JU) ROM the template loads to play (scripts/web/build-lift.sh). The tests are `gp_theme::same_as::tests` (made-up folders) and `gp_web_lift::tests::a_shared_theme_is_offered_on_both_models` (two made-up versions of Rivet Rodeo's template sharing one paper theme, each lifted, the two looks byte for byte the same).

## Large-art rules

A look's `look.txt` may name a rules file with `art FILE`: a large-art look, whose pictures are drawn bigger than the game's own (lex00/turnips is one). The rules are text, `gp-art 1` on the first line and one rule a line, read by `gp_present::art` in the game's module, so the page and any runtime built on gp_present draw the same frame; site/src/content/docs/themes.md lists every rule for theme makers, and the module's own comment holds the details. The rules a page reads changed over time in one way only: lines added since `gp-art 1` began start with `#+`.

A reader takes a line whose first word starts with `#` for a note. So a page built before a `#+` line passes over it and draws the look without it, where an unknown line of any other kind makes the page leave the whole look out ("its rules don't read"). A reader that knows `#+` lines passes over one it can't read the same way and keeps it in `ArtRules::skipped`, and `gp theme look check` names it as a problem by its line. New rules go in as `#+` lines, so a look that uses one stays offered on every page.

| Line | Means | Default |
|---|---|---|
| `#+ hud_over` | The HUD's layer and the text in its background are drawn after the bodies (after their fading ghosts, before the dimmed wall, the props and the HUD's sprites), so a body standing on a HUD row doesn't cover its text | Without the line they are drawn under the bodies, as every look before it |
| `#+ hud_over ROW...` | Only those rows of the HUD, each 8 pixels from the screen's top (row 2 is y 16 to 24, as a HUD field's `at` numbers it); the layer is cut into bands of rows by clips, the text goes by its row. Several lines add up; one with no row takes the whole HUD | |
| `#+ in REGION pic BUILD ART W H DX DY` | A `pic` rule that holds only while the body is in the named region of the stage: the middle of its picture's bottom row inside the region's box, on the region's level | A body shows its other pictures |
| `#+ in REGION timed BUILD FROM TO ART W H DX DY ...` | A `timed` rule likewise, with the same options | |
| `#+ room SCREEN...` | An arcade game's screens built from the stage's pieces on their own cells (below, "Rooms"); the lines after it, up to the next `#+ room`, are those screens'. Needs the `stage` line before it | The board's own screen, or a `pic` line's picture of it |
| `#+ room_cells CLASS TEST...` | A class of the screen's cells: those for which every test holds | |
| `#+ room_fill CLASS PIECE` | The piece repeated over the class's cells at its own size, cut to them | |
| `#+ room_run CLASS LEFT MID RIGHT` | On each group of the class's cells, the two end pieces at its ends and the middle one repeated between, as tall as the group | |
| `#+ room_box CLASS PIECE` | The piece over each group of the class's cells | |
| `#+ room_at CLASS PIECE DX DY` | The piece at its own size at each of the class's cells, moved `DX DY` screen pixels | |

In a region a body's `in` rules come first, its timed ones that hold now and then its picture, before its other `timed` rules and its `pic`. A region's name is lowercase letters, digits, `-` and `_`; the regions are named in the game's level kit (docs/level-kit.md, "Regions of a stage"), never by coordinates in the theme. `gp theme look check` refuses an `in` line whose region the kit doesn't name; the build carries the kit's regions in `preview.json` and the page gives them to the module (`Art::set_regions`). Moss Hop's kit names one, `start`; Donkey Kong's and Tapper's name none yet. The art an `in` line names is numbered after every other art file, so a look without the lines numbers its files as before.

The HUD's sprites (a field drawn in a sprite palette, a HUD sprite) are drawn last in every look, over the bodies, so `#+ hud_over` changes nothing in a game whose HUD is all sprites (Moss Hop's); `gp theme look check` warns of that.

### On an arcade game

An arcade game drawn by its board's art (docs/formats/board-art.md, "Themes") takes the same rules, read and painted by the same `gp_present::art`, in the page's `gp_board.wasm` instead of the game's module. A rule names the board's pictures in place of the build's: a screen by its `board-screen` slot (`screen_saloon`), a sprite as `sprite-ENTITY-CODE` with the picture code in hex (`sprite-mug-4d`), a counter's digit as `counter-COUNTER-D` and a repeated tile as `counter-COUNTER`. Sizes and places are the board's picture's pixels (512 by 480). No rule line is new; the names are new values in the lines there were. A page built before them doesn't draw such a theme and draws nothing wrong: its arcade lift offered no finished theme, and its game's module refuses rules that name none of its pictures. A page with the board's names offers the theme in Settings, Theme and draws it.

#### Rooms

A theme never holds a picture of a room, as it never holds one of an NES stage: `#+ room` lines build a screen from the theme's pieces on the layout the player's own lift gives (`gp_present::art::room`). When the page's `gp_board.wasm` loads the rules it hands each named screen to the room as a grid of 32 by 30 cells (`BoardArt::screen_cells`), each with what the board draws there: the cell's tile code (0 to 1023), its colour bank and priority (0 to 3 each, from the screen's word), and the colours of its 64 pixels in the colours of the first look that shows the screen (the level's own, then its `looks` in order). The rules sort the cells into classes by those and lay pieces on each class. Nothing in the rules is a place on the screen, so the room has the original's layout and holds none of it.

```text
gp-art 1
scale 2
stage 2 40 30 60 20 16 30
stage_piece plank pieces/plank.png 64 32
stage_piece cap pieces/cap.png 24 32
stage_piece boards pieces/boards.png 64 64
stage_piece paper pieces/paper.png 128 128
stage_piece door pieces/door.png 40 64
#+ room screen_saloon
#+ room_cells door r>g+60 g>b+40 max>0+200 share=25
#+ room_cells counter r>b+40 r>g+20 share=40 !door
#+ room_cells floor min>max-12 max>0+60 !counter !door
#+ room_cells wall !counter !door !floor
#+ room_fill wall paper
#+ room_fill floor boards
#+ room_box door door
#+ room_run counter cap plank cap
```

| Line | Means |
|---|---|
| `#+ room SCREEN...` | The rules after it are those screens' (`board-screen` slots, as a `pic` line names a screen), up to the next `#+ room`. A screen two rooms name is the first's. It needs the `stage` line before it: the room is drawn in the stage's pixels (`stage SCALE ...`, SCALE to a board pixel) over its backdrop gradient, with the pieces its `stage_piece` lines give |
| `#+ room_cells CLASS TEST...` | A class of cells (its name lowercase letters, digits, `-` and `_`, once a room). A cell is in it when every test holds. `tile=N` or `tile=LO-HI`: its tile code, decimal or `0x` hex. `bank=N`, `prio=N`: its colour bank, its priority. Several `tile=`, `bank=` or `prio=` tests hold when any one does. `!CLASS`: not in a class given before. A colour test as a stage's `floor` takes it (`A>B+K`, each side one of `r g b min max 0`) holds for the cell when at least `share=P` percent of its pixels (50 when not given) pass every colour test. A class with no test holds every cell |
| `#+ room_fill CLASS PIECE` | The piece repeated at its own size, its `stage_piece` `W H` in stage pixels, on a grid from the screen's top left, over the class's cells and cut to them: a wall, a floor, a sky. A piece one cell square (16 board pixels times the scale) goes cell by cell instead, mirrored where the board mirrors that cell's tile, so a class for each tile code and a piece for each skins a room tile for tile |
| `#+ room_run CLASS LEFT MID RIGHT` | Each group of the class's cells (cells side by side or one above the other) is a run over its box: `LEFT` at its left end and `RIGHT` at its right end, each as tall as the box and as wide as its art's shape makes it, and `MID` repeated between them the same way, cut there. `-` for none. Ends wider than the box share it. A counter, a bar, a shelf |
| `#+ room_box CLASS PIECE` | The piece stretched over each group's box: a door, a tap, a sign. A PIECE written `a|b|c` (in any room rule) is one of them, each as likely, picked by a seed the page gives each time it loads the theme (`gp_board_room_seed`), the same on every screen of the room. A rule ending `pick=NAME` (a name as a class's) picks by NAME in place of its list's text, so lists of the same length with the same NAME, in any room, pick the same place in each: `#+ room_at frame bar01|bar02 0 0 pick=painting` in the bars and `#+ room_box wall wide01|wide02 pick=painting` in the bonus room hang the same painting |
| `#+ room_at CLASS PIECE DX DY` | The piece at its own size (its `stage_piece` `W H`) at each of the class's cells, its top left `DX DY` screen pixels from the cell's, either one negative if need be. A class of the one tile each door or cask has once, its anchor, places each whole door or cask where the original draws it, even where its cells touch the next one's, which would make one group of them for `room_box`. Where the board mirrors the anchor's tile left to right, the piece and its offset are mirrored too, so a room the game draws mirrored needs no rules of its own |

The pieces are drawn in the order of their lines, the last on top. A room is drawn in place of its screen as the stage over an NES level is: under every sprite, with the counters' cells the theme doesn't draw still the board's, and in place of any `pic` line for the same screen. A screen with no room keeps the board's own picture or its `pic` line. `gp_board_load_art` gives each room's draws beside the files (`"rooms": [[pic, stage], ...]`, a stage as `gp_art_stage` gives one), the page paints each into its canvas once, and the frame's `["stage", pic]` draw places it. A sprite of the board (Tapper's bar ends and taps are sprites, not cells) is themed by its `sprite-` name, not by a room.

The example was tried on Tapper's saloon from the owner's lift on 2026-11-16 (outside any repo): the doors, the four counters, the floor between them and the back wall each came out as their own class, with the floor's slanted left edge a staircase of cells. The taps on the right wall and the sign at the top draw the counters' browns, so they went to `counter` too and each got a run of its own; a theme that wants them drawn otherwise tells them apart by `tile=` from its own sheet. A cell is 16 board pixels square, so a room follows the screen's layout in steps of a cell: a slanted edge, like the floor of Tapper's saloon running back to the wall, is a staircase of cells, and a cell that is part floor and part wall goes to whichever class its `share` gives it. The tests read only what the lift holds, so a game whose screens draw two features with the same tiles and colours needs a theme that tells them apart some other way, which these lines don't have yet: a cell's neighbours, its row against another class's, or a region the game's kit names. `gp theme look check` builds every room as the page does and names, as a problem, a `room_fill`, `room_run` or `room_box` line whose class found no cell on a screen the game shows (it draws nothing there), and a piece no `stage_piece` line gives.

## Stacks

```toml
format = "0.1.0"
model = "mosshop"

[[layer]]
theme = "paper"
scope = "baked"
enabled = false

[[layer]]
theme = "dusk"
scope = "baked"
```

| Key | Meaning |
|---|---|
| `format` | `"0.1.0"` |
| `model` | Optional: the model it's for |
| `layer.theme` | A theme id |
| `layer.scope` | `baked`: shipped with the template or fork as its default look, in its `themes/<id>/`. `personal`: only on this player's screen, in `$GP_HOME/themes/<id>/`, never shipped |
| `layer.enabled` | Default true |

A template's baked stack is `themes/stack.toml` and holds baked layers only. A player's stack for a model is `$GP_HOME/themes/stacks/<model>.toml` and holds personal layers only. `--themes` uses both, baked first.

### Stacking

`gp_theme::stack::resolve` applies the rules in this table.

| Rule | Detail |
|---|---|
| Order | Only enabled layers count. Baked layers come first in their order, then personal layers in theirs, so a player's themes sit on the template's look whatever the file's order |
| Where | A baked layer's theme must be in the template's `themes/` (its folder may be a `same_as.toml` pointer, above). A personal one is looked for in `$GP_HOME/themes/` first, then the template's |
| Once | A theme may appear once in a stack |
| Models | A stack naming a model, or a theme with `for`, must name the model it's applied to |
| Palette rules | Compose bottom to top: each layer's most specific rule for the pixel's role applies to the color the layers below made |
| Effects | Of the same kind and target, the highest layer's wins. Particles of different targets or styles all draw |
| Restyles | The highest layer with one for a role or slot wins, and lower layers fill what it lacks (the Minecraft resource pack rule, in [the stubs research](../research/asset-stubs.md)). A restyled pixel takes palette rules only from the layers above the one that restyled it |

The model's own art, original or stub, is the bottom of every stack.

## The readability check

`gp_theme::readability` measures the themed pictures by role, after the screen tint, and refuses or warns. Scanlines and vignettes darken foreground and background alike, so they aren't counted.

The background is every pixel with a `world` role, plus each level's backdrop color. Its dominant colors are the fewest most common ones covering 95% of it (at most 16).

| Group | Roles |
|---|---|
| player | `player` |
| enemies | `enemy`, `boss` |
| hazards | `hazard`, `projectile` |
| text | `hud`, `font`, `menu` |

| Measure | What it is |
|---|---|
| `distinct` | The share of the group's pixels at least 20 (CIE76 delta E) from every dominant background color |
| `contrast` | The WCAG 2 contrast ratio of the group's median-luminance color with the most common background color |
| Pair `delta_e` | The distance between two groups' mean colors |

| Verdict | When |
|---|---|
| refuse | A group under 25% distinct |
| warn | A group under 50% distinct; a contrast under 1.5 with under 75% distinct; two groups within delta E 10 of each other |
| ok | Otherwise |

A group the model has nothing of is listed as absent (Moss Hop has no text). `gp theme video` and `gp-web-prep` refuse a stack the check refuses, unless `--allow-unreadable`; `gp theme check` exits 1.

The thresholds are first guesses, set so Moss Hop's two themes pass and a theme that tints everything one gray is refused.

## Style checks

`gp theme check` measures each sprite the contract holds to Style against the style guide, through the `StyleMeasure` trait. `Deterministic` is the only measure now; an embedding model's distance is meant to sit beside it later, behind the same trait.

| Metric | Per | What it is | Passes when |
|---|---|---|---|
| `palette_distance` | Sprite | The mean CIE76 distance from each opaque pixel to the nearest palette color | At most `max_palette_distance` (default 12) |
| `edge_density` | Sprite | The share of opaque pixels with a 4-neighbour of another color or transparent | Within `edge_density`, when given |
| `size` | Sprite | How far the opaque pixels' box is from the entity's size, as a share of it | At most `size_tolerance` (default 0.25) |
| `size_consistency` | Role | The spread of `size` across the role's sprites | At most `size_tolerance` |

`gp theme check` exits 1 when a measure fails.

## Where themes are drawn

| Runtime | How |
|---|---|
| Generator pictures (`gp_look::present::art_themed`) | Restyles, then palette rules, gradients and glows, into each level picture, each sprite frame in each palette set and the HUD's pictures; a cycling color's tint by the `sky` role's palette rules; the overlay as `overlay.png`; the emitters and each entity's role for particles |
| Bevy build | The themed pictures; the overlay on the camera at z 50, at three overlay pixels to a game pixel with a CRT or a grid, one for each window width the widescreen option draws (`overlay.png`, `overlay-<width>.png`), and hidden while the player turns screen effects off ([the generator](../generator.md#picture-options)); particles as small sprites on the camera, from the copied `particles.rs`, made again each frame drawn. The sim crate is byte for byte the same with any theme |
| Web preview | The themed pictures; `preview.json`'s `theme` names the overlay and the emitters, and `preview.js` draws them after the sprites |
| `gp theme video` and the editor's preview | The same pictures, composed on the CPU (`gp_theme::compose`) from the interpreter's world each frame |
| `gp video` | A recording's picture through `*` rules, screen particles and the overlay, after the frame is checked against the recording |

The proofs that a theme can't change the simulation: `gp theme video` prints the digest of the per-frame state hashes, which `crates/gp-cli/tests/themes.rs` holds equal to `gp interp run`'s with each Moss Hop theme; the same test holds the generated sim crate byte for byte equal with and without a theme. `just check-cycle` also runs `gp gen conform --themes` (the themed Bevy build against the interpreter, every frame) and `just web-mosshop themes` (both web builds with the theme, every frame's hash in the browser).

## Moss Hop's themes

| Theme | Tier | What it does |
|---|---|---|
| `templates/mosshop/themes/dusk` | Live effects only | A sunset gradient sky, cool dim ground, a brighter player with a warm glow, red-leaning enemies, fireflies, a light tint and a vignette |
| `templates/mosshop/themes/crt` | Live effects only | A television's line gaps, aperture grille and dark corners (`crt`, strength 0.7) |
| `templates/mosshop/themes/paper` | Restyled assets | Paper cutouts painted by `gp theme paint` from the model's roles: a palette change for `player` and `enemy`, a cell for `sky`, `terrain` and `goal`, as `.pix` files under CC0-1.0, with a style guide of the painter's colors |

Moss Hop's stack ships dusk on, and paper and crt off.

`gp theme paint` knows nothing about Moss Hop. It paints a sprite for each entity role the model has, at the size of the role's largest sprite, and a cell for each item role of each metatiles slot. Shapes and colors come from the role alone (a round creature for the player, a low one for enemies, spikes for hazards, a diamond for collectibles, a torn strip for terrain).

## Versions

| Version | Changes |
|---|---|
| 0.1.0 (2026-10-28) | First version |
| 0.1.1 (2026-10-29) | Added `audio`, the restyled-audio tier. A 0.1.0 theme reads as before; a theme that uses `audio` names 0.1.1 |
| 0.1.2 (2026-10-30) | Added the screen effects `crt` and `grid`. A 0.1.0 or 0.1.1 theme reads as before; a theme that uses them names 0.1.2 |
| 0.1.3 (2026-11-03) | Added `sound` (the feature: a file per sound role), a restyle's `entry` (a layout entry as a picture) and `.pix` for a tiles slot's restyle. A 0.1.0 to 0.1.2 theme reads as before; a theme that uses them names 0.1.3 |
