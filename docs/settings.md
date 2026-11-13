# The settings screen

Every game Game Printer builds has the same settings screen, inside the game. It lists the modes and every switch of the game's `profiles.toml` ([the format](formats/profiles.md), [profiles](model.md#profiles)), and the game draws it itself, in the interpreter preview that the public page and the mechanic page's PLAY tab play ([the web build](web.md)). The user asked for it on 2026-11-03: "we need a settings screen that should be standardized somewhat as all our games will have them, with all the toggles. there need to be modes for the toggles, at the very least "retro" and "normal". retro is the frame accurate version. the normal version would not model any lag or the like into the game", and "the settings panel has to be in game". This page came from game-printer; the generated Bevy game, the screen's second host there, was dropped, so the web preview is the one host.

One piece of code draws it and reads its keys: `gp_present::settings` (`crates/gp-present/src/settings.rs`). gp-web's exports call it (`gp_settings_open`, `gp_settings_key`, `gp_settings_draw`, and `gp_settings_saves`, `gp_settings_shell`, `gp_settings_themes` and the others in `crates/gp-web/src/abi.rs`), so the screen is the same picture, with the same keys, wherever the preview runs.

## Opening it

| Where | Keyboard | Controller |
|---|---|---|
| Any game, while playing | Esc opens and closes it. F1 does the same | The right stick's press (`RightThumb`, button 11 of the browser's standard layout) |
| The public page | The same, or the Settings button in the top bar ([the public page](web.md)), which closes the screen too when it is open | The same |
| The mechanic page | F1, or the settings button, in free play | The right stick's press in free play |

On 2026-11-03 the user wrote "F1 triggers my system F1 so the browser seems to not pick it up". On macOS F1 to F12 are brightness and media keys unless fn is held, so Esc is the main key and F1 stays as an extra. While the game plays, no NES button and no shell action is on Esc or F1, or on the right stick's press, so the same keys and button work in every game. The mechanic page had a job for Esc already and keeps it: in free play Esc goes back to HELP ([the mechanic page](mechanic-page.md)), so the page opens the screen only through F1, the right stick or its button. While the screen is open, Esc closes it and doesn't leave free play. While the screen is open the game holds: no frame runs, and on the mechanic page the original holds too.

| Key | Keyboard | Controller |
|---|---|---|
| Move | Up and Down (W, S) | D-pad up and down |
| Change the mode, or turn a switch on or off | Left and Right (A, D), or Enter, Space, X | D-pad left and right, or A |
| Choose a save slot, or what to do with it | Enter, Space, X | A |
| Close | Esc, F1, Backspace or Z, or Done. With a slot chosen, Esc, Backspace or Z go back to the list | The right stick's press, or B (back to the list with a slot chosen) |

A theme chosen here is the game's only once it has loaded whole: its pictures and its present text are read first, then the module takes the text, and only then are they drawn and kept for the next restart (`applyLook` in `scripts/web/page/preview.js`). Before 2026-11-04 the text was kept before the module took it, and a failed fetch could leave the game with no presentation, so it froze (the user: "Switching from Normal back to Retro and then closing Settings, the game freezes"). The public page's check (`scripts/web/public/check-page.mjs`) plays the theme and both modes in each ROM, each closed with the game stepping on. The Settings button pressed while the screen is open closes it.

## What it shows

The screen is drawn at the game's own resolution (256 by 224 on Donkey Kong), in whole pixels, in the shell's 5 by 7 font, over the game a little see-through:

| Line | What |
|---|---|
| SETTINGS | The title |
| Mode | `< retro >`, `< normal >`, another preset's name, or `< custom >` for a set no mode names |
| Theme | `< Original >` and the other looks the game found, then the themes published for the game, when there is more than one. `Original` is the ROM's own art, and is listed only where the ROM's assets were lifted. A change draws the next frame in the look chosen, pictures and sound, with no restart, since the simulation never reads it; a published theme not yet downloaded is downloaded first and every file checked |
| Under the Theme line | For a published theme: `by AUTHOR, downloaded`, or `by AUTHOR, download 2.1 MB`. The cursor passes over it |
| Remove it from the cache | For a downloaded theme: takes its files out of the browser's cache. It can be downloaded again |
| `[ ] Console only` | The shell's switch for no saves, loads or rewind ("Saves on the screen", below) |
| `[ ] Quick save buttons` | The shell's switch for quick save and load on a controller's bumpers |
| Saves | A heading, then Resume (once the autosave is kept), Slot 1 to Slot 4 and Load from a file |
| A heading for each group | The group's `title`, in the file's order. The cursor passes over it |
| `[X]` or `[ ]` and a switch's name | Each switch of the group, on or off |
| Done | "Done: restart in these settings" once the switches differ from the ones the game runs in |
| Cancel the changes | Once anything changed: back to the game in the settings it runs in |
| The cursor's line explained | Under the list: the mode's description, a group's or a switch's `description` from `profiles.toml` |
| The keys | At the bottom |

The list scrolls to keep the cursor in view. Turning a switch on or off makes the set custom unless it is exactly a mode's. Choosing a mode turns on exactly its switches.

## Modes

| Mode | What it is |
|---|---|
| `retro` | The model as written, every switch off: frame for frame what the console does. It is always there and `profiles.toml` can't define it. Every comparison, score and refinement runs it |
| `normal` | The game with no lag and none of the console's quirks. Every game has it. It turns on every switch of the `timing` group and the game's fixes. It may turn nothing on (feature `preset.empty_normal`) when the model has nothing to turn off, as for Moss Hop, and then plays as `retro` does |
| Any other preset | Such as Moss Hop's `assist`, after `normal`, by name |

| Game | `normal` |
|---|---|
| Donkey Kong (`donkey-kong`) | 12 switches: `rng_per_frame` (timing) and the 11 engine fixes |
| Moss Hop (`mosshop`) | Empty: no quirk options and no timing switch |
| Tapper (`tapper`) | Empty |

## The timing group

The group named `timing` (title "Timing") is the standard place for a switch that turns off frame timing. Frame timing covers lag frames and slowdown, and logic frames the game skips when it is busy. It also covers anything a frame's spare time decides. `normal` turns on every switch in it.

On 2026-11-03 no model had a lag frame, slowdown or skipped logic frame to turn off, and that still holds. Every model frame runs every system in the spec's order, and a model frame is one frame on which the original reads the controller: `gp interp compare` skips the original's lag frames when it lines the two up ([frame pairing](interpreter.md#frame-pairing)), so the model never runs one. Donkey Kong's original overruns a frame on some busy ladder grabs, and the mapping reads its RAM at the next NMI (`align.read_at_nmi`) rather than the model running a late frame; [cycles](cycles.md) lists the frame-overrun switch as not started, since no frame overran in the user's play. The one timing behaviour any model has is Donkey Kong's random register, which steps once for each pass of the idle loop the frame's spare time allows (the hook `dk_idle`, `ext/src/idle.rs` in the `donkey-kong` package). Its switch `rng_per_frame` is in the timing group, and `normal` has it.

## Changing a setting

Switches change the model before it runs (`gp_model::profile::Profiles::apply`, called by gp-web's `profile_bundle`), never a game under way. So when the screen closes with switches other than the ones the game runs in, the game starts again from power-on in the new set. Closing with the same switches, or picking a mode that has the same ones (Moss Hop's empty `normal` from `retro`), keeps the game where it is and only keeps the new name. Switches never change mid-game. A run, a replay and a save each hold one set, and the state hash covers it. A restart empties the rewind ring, and a save slot of another set starts the game again in that set first ("Saves on the screen", below).

## Where it is kept

The setting is kept per game, by the mode's name (`normal`) or the switches on joined by `+`. It is never read by the simulation: it is a name the runtime applies to the model before power-on.

| Host | Where |
|---|---|
| The preview | `localStorage` keys `gp-settings:<model>` and `gp-theme:<model>`, read for keyboard play only. A `profile=` or `theme=` in the URL wins, and test, `run=` and control mode never read them. Console only and Quick save buttons are the shell's, kept in its storage as `settings/shell.txt` ([the runtime shell](shell.md#the-shared-parts)) |
| The mechanic page | The same key, under the model's name from the build's `preview.json`. The page passes it to free play's rebuild as `profile=`. Scores always run `retro`, whatever the page plays |

## Saves

The preview's save slots hold one record, `gp_sim::save` (`crates/gp-sim/src/save.rs`). The research behind it is `$GP_HOME/work/workers/saveR/quick-save-options.md` (2026-11-04, in game-printer's home), whose first step the user approved that day.

| Field | What |
|---|---|
| `GPSV`, then version 2 (u32) | The record. Version 1 had no digest |
| The model's name | The game it is of |
| The spec digest | The build that made it: BLAKE3 of the spec in the save's profile, as the preview's `gp_spec_digest` gives it |
| The profile | The mode the game was played in, which the state leaves out |
| The canonical state | `World::canonical`, to the end of the record |

Each string is a u16 length and UTF-8, and numbers are little-endian. A theme is never in a save, so a save loads in every look.

A build loads only its own saves. The canonical state's version follows its layout, not the model, so a save from before a model change could read as a state of the new model and be wrong. The digest names the build exactly, and any other is refused with "Can't load a save from another version". A version 1 save is refused the same way. A save of another game gets "Can't load a save of another game", and bytes that aren't a save "Can't load this save: it is damaged". Each fits a message line on a 256-pixel screen.

A slot is kept in the runtime shell's storage ([the runtime shell](shell.md#storage)): IndexedDB, `<model>/saves/slot-N.gpsave` (`preview.json`'s `model`). From 2026-11-04 to 2026-11-05 it was the `localStorage` key `gp-save:<model>:slotN`, and before that the build's digest (`gp-shell:<digest>:slotN`, the state and profile as JSON), so every page update hid the slots made before it. A slot under either old key is moved into the storage on the first visit ([saves kept before the standard](shell.md#saves-kept-before-the-standard)).

The ROM player's save (`gp_play`, `crates/gp-play/src/lib.rs`) is another format with its own header, `GPPS`. It shared `GPSV` until 2026-11-04 and still reads states kept with that header.

## Saves on the screen

The user approved the save features on 2026-11-04 and 2026-11-05 ([the runtime shell](shell.md#saves-on-the-settings-screen)). The screen shows them where the host gives it its slots (`Screen::with_saves`) and the shell's settings (`Screen::with_shell`), as the web preview does.

| Line | What |
|---|---|
| `[ ] Console only` | No saves, loads or rewind, as on the console. A switch of the shell, never of the model, so it never restarts the game; kept beside the theme. While it is on the Saves line says "Off: console only is on" |
| `[ ] Quick save buttons` | On a controller, the right bumper saves and the left bumper loads, to the last slot used (slot 1 at first). Off at first, so a stray press can't replace a save |
| Resume | The autosave, written as each level starts. Listed once it is kept. It can be loaded, downloaded and deleted, not saved to |
| `Slot 2  3:41  Normal  5 min ago` | A slot: its play time (the state's frame at the game's frame rate), its mode, and how long ago it was saved (a stamp the host keeps beside it, from its clock). An empty slot says `empty`, a save of another version `another version`, and bytes that are no save `can't be read`. The line under the list adds its level, when the game has more than one ("Slot 2: level 2, 3:41 played, in Normal, saved 5 min ago") |
| Load from a file | A save downloaded before, picked with the browser's file chooser |

Choosing a slot shows what can be done with it, under its name. An empty slot offers only Save here and Back.

| Choice | What |
|---|---|
| Save here | Saves the game where it is into the slot ("Save here, replacing it" when the slot is kept) |
| Load | Loads the slot and goes back to the game |
| Download as a file | Hands the slot over as a file, `<game>-slotN.gpsave` |
| Delete | Deletes the slot and its stamp |
| Back | Back to the list |

The screen hands the choice to the host, which does it through its storage and says how it went under the list: "Saved to slot 2", "Slot 2 deleted", "Slot 1 downloaded as mosshop-slot1.gpsave", or the sentence a refused save gets. A load closes the screen onto the save; the switches the game runs in stay, since a restart would undo the load. A save in another mode starts the game again in that mode for this play first, and the mode kept stays. A mode chosen on the screen before the load is dropped; a theme chosen is drawn.

## Messages in control mode

The mechanic page runs the preview in control mode, in a hidden frame, and copies its picture, so the screen the page shows is the preview's own. It opens the screen with `{gp: "settings", key: "open", profile}` and then sends each key as `{gp: "settings", key}` (`up`, `down`, `left`, `right`, `choose`, `back`, `open`). The preview answers `{gp: "settings", open: true}`, and on closing `{gp: "settings", open: false, profile, restart, theme}`; the page keeps the setting and, when `restart` is true, starts free play again in it. The preview has already drawn the look `theme` names. `{gp: "theme", id}` picks a look without the screen, answered by `{gp: "theme", id}` with the look shown. The public page's Settings button sends `{gp: "settings", key: "open"}` the same way.

## The standard's checks

| Rule | Checked by |
|---|---|
| A game in the tool repo's `templates/` has `profiles.toml`, with a `normal` preset that turns on every switch of the `timing` group | `every_template_keeps_to_the_settings_standard` (gp-model, `Profiles::standard_problems`) |
| The file's switches and presets are sound against the model | `gp model check` ([the format's rules](formats/profiles.md#rules)) |
| Only `normal` may turn no switch on | `gp model check` |
| `gp game new` writes a `profiles.toml` with an empty `normal` and a comment saying what goes in it | `crates/gp-cli/src/game_new.rs` |

| Test | What it checks |
|---|---|
| `gp_present::settings::tests` | The Saves line, the slot chosen and what can be done with it, a load's close, and Console only (listed in [the runtime shell](shell.md#the-standards-checks)). The Theme line: listed with two looks or more, stepped, and closing with the look chosen and no restart; a published theme's author and state under it, and its removal from the cache. The modes' order, a mode setting its switches, a toggle making a custom set, Cancel, an empty `normal`, a kept name the game lacks, the picture at 256 by 224 and every line in reach on a short screen |
| gp-web `settings_tests` | Donkey Kong's file on the preview's screen (`the_preview_screen_reads_a_templates_profiles`: the timing group first, 12 switches, retro then normal, and normal restarting; ignored unless the games checkout is beside the repo); every template's file gives retro and normal (`every_templates_screen_has_retro_and_normal`) |

Not here yet: game-printer's `headless --settings-picture FILE`, which wrote a PNG of the screen from the generated game, went with the Bevy generator.
