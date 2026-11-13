# The runtime shell

Every game Game Printer builds has the same shell around its simulation. It keeps save slots and a rewind ring, writes the autosave, and reads the keys and gamepad buttons that drive them. It is a standard in the settings screen's pattern ([the settings screen](settings.md)): one shared piece of code for each concern, and a target supplies only its storage. The user asked for it on 2026-11-04, approving the seven save features of the research (`$GP_HOME/work/workers/saveR/quick-save-options.md` in game-printer's home, design B): "think about how these can all be standard for game printer targets". This page came from game-printer. The generated Bevy game, its second target, was dropped, so the web preview is the one target; the shared code still names a Bevy target in places, and nothing reads those parts.

A target is a way to play a built game. The web preview, which the public page and the mechanic page play ([the web build](web.md)), is the shipped game and has the shell.

Every part keeps the game as its canonical state (`gp_sim::World::canonical`, the bytes the runtime protocol's `load-state` takes) and puts it back whole between ticks. So none of them changes what a tick does: a run that saves, loads and rewinds reaches the state hash a straight run of the same input reaches.

## The shared parts

| Concern | Code | What it is |
|---|---|---|
| The save record | `gp_sim::save` | `GPSV` version 2: the model's name, the build's spec digest, the profile and the canonical state ([saves](settings.md#saves)). A build loads only its own saves |
| Slot names | `gp_present::shell::slot_name` | A slot is the storage name `saves/slot-N.gpsave`. Slot 0 is the autosave (`AUTOSAVE_SLOT`); the player's are 1 and up. The number keys reach slots 1 to 4 (`QUICK_SLOTS`). A slot handed to the player as a file is `<game>-slotN.gpsave` (`export_name`) |
| The rewind ring and its policy | `gp_present::shell::Ring` | A state every 2 frames (`REWIND_EVERY`), 20 seconds of them (`REWIND_SECONDS`, `Ring::standard`). Held rewind steps back one state a tick, so 2 frames a tick. The lengths a player can choose are 0 (off), 10, 20, 40 and 60 seconds (`REWIND_CHOICES`). A load or a restart empties it |
| The autosave policy | `gp_present::shell::autosaves` | Slot 0 is written after a tick whose events hold `level.started` |
| The slots as a player sees them | `gp_present::shell::slots` | `write_slot` keeps a record with a stamp beside it, `saves/slot-N.when` (the seconds since 1970 when it was saved, as text; the shell's clock, never the simulation's). `slot_lines` reads slot 0 and 1 to 4 for the settings screen's Saves line: each one's frame, mode and level from the record and its state, its stamp, and whether this build refuses it ("Saves on the settings screen" below) |
| The shell's own settings | `gp_present::shell::Settings` | "Console only" and "Quick save buttons", both off at first, kept through storage as `settings/shell.txt` (a `name = true` or `false` line each). They are the shell's, never the model's, so a change never restarts the game. `allows` says whether a binding of the key table acts under them |
| Keys and buttons | `gp_present::shell::keys::BINDINGS` | One table of every key and gamepad button the shell and the NES buttons use ("Keys and buttons" below) |
| Storage | `gp_present::shell::storage` | The one thing a target supplies ("Storage" below) |
| The shell check | `gp_present::shell::check` | What every target must pass ("The standard's checks" below) |

The code is `crates/gp-present/src/shell/`. gp-web, the preview's module, links gp-present.

## The web preview

| Concern | How the preview does it |
|---|---|
| Storage | `scripts/web/page/storage.js`: IndexedDB database `gp-saves`, store `files`, key `<game>/<name>`, the game being `preview.json`'s `model`. Where IndexedDB can't be opened, localStorage as base64 under `gp-file:<game>/<name>` |
| Saving | The settings screen's Saves line, Shift+1 to Shift+4, and the right bumper with Quick save buttons on |
| Loading | The Saves line, 1 to 4, the left bumper with Quick save buttons on, and Load from a file. A save in another mode starts the game again in that mode for this play, then loads; the mode kept in the settings stays |
| Autosave | Slot 0 at each level start (`gp_autosave_due`), listed as Resume |
| Rewind | Hold Backspace or the left trigger. The ring is the module's (`gp_ring_reset`, `gp_ring_note`, `gp_ring_back`, `gp_ring_clear`) |
| The Saves line, Console only, Quick save buttons | Drawn on the settings screen (`gp_settings_saves`, `gp_settings_shell`) |
| Its own save and load | `gp_save_record` and `gp_load_record` (`Preview::save_record`, `Preview::load_record`) |

### Saves kept before the standard

The web preview kept its slots in localStorage until 2026-11-05. The first visit after that moves them into the shell's storage and removes their localStorage keys. A save record kept in base64 as `gp-save:<model>:slotN` (2026-11-04 to 2026-11-05) moves as it is. A state and profile kept as JSON under `gp-shell:<digest>:slotN` (before 2026-11-04) becomes a record when the digest is this page's and the profile is the one played; any other stays where it was. A slot already in the storage was saved later, so it stays and the older one is dropped.

## Storage

The storage interface is the WIT package `game-printer:shell@0.1.0`, interface `storage` (`WIT_PACKAGE` in the bindings): what a target keeps for one game. A name is a path relative to the game's own storage: lowercase letters, digits, `-`, `_`, `.` and `/`, with no `..` (`gp_present::shell::valid_name`). Another game's files are never seen.

| Function | What it does | The web preview |
|---|---|---|
| `list(prefix)` | The names kept that start with `prefix`, sorted | A key range of the store |
| `read(name)` | The file's bytes, or none | Gets the key |
| `write(name, bytes)` | Keeps the bytes, whole or not at all | One transaction |
| `delete(name)` | Removes the file; a name not kept is no error | Deletes the key |
| `persist()` | Asks that the files stay when the system wants room; true when they are kept for good | `navigator.storage.persist()` |
| `export(name, file-name)` | Hands the file to the player, and says where it went | Downloads it |
| `import()` | A file the player picks, or none | The browser's file chooser |

The bindings are `crates/gp-present/src/shell/storage.rs` (the `Storage` trait and the `Picked` record) and `scripts/web/page/storage-wit.js` (each function's name and parameters, and `checkStorage`, which the preview runs on its host before using it). A browser host answers each function with a promise: an `option` is the value or null, and a `result`'s error rejects it. `gp_present::shell::Memory` keeps the files in memory, for the native checks.

`gp self shell` (game-printer's `gp docs shell`) writes both bindings from `wit/shell/storage.wit` with `wit-parser`, and `scripts/web/page/shell-keys.js` from the key table; `just docs` runs it, and `gp self shell --check` exits 1 while one is stale. The test `the_shell_s_generated_files_are_current` (crates/gp-cli/src/docs_shell.rs) fails while any of the three isn't what it writes, so a change to the key table or the interface is made in the WIT or the Rust table, and the JavaScript follows.

The preview calls `persist` on the player's first save of a visit (an autosave doesn't ask), `export` from a slot's "Download as a file" and `import` from "Load from a file" on the Saves line. A browser opens its file chooser only while a key or a click is being handled, so Load from a file needs a key or a click; a gamepad's button can't open it.

## Keys and buttons

One table, `gp_present::shell::keys::BINDINGS`, names every key and button, by action. A key is its `KeyboardEvent.code` (`KeyX`, `Digit1`, `F5`). A button is named as Bevy's `GamepadButton` variant was, placed in the browser's standard gamepad layout by `STANDARD_BUTTONS` (`RightThumb` is button 11). Each binding says when it acts (in play, or while a shell screen is open), which targets have it (`both`, `web` or `bevy`; the preview reads `both` and `web`), and what it needs of the shell's settings (`needs`): `always`, `not-console-only` (saves, loads and rewind, off while Console only is on) or `quick-buttons` (only with Quick save buttons on).

The preview reads it as `scripts/web/page/shell-keys.js`, with the rewind policy and the slot names. preview.js takes the NES keys, the gamepad's buttons, the settings screen's keys and every shell key from it.

| Action | Keys | Buttons | In the preview |
|---|---|---|---|
| Controller 1, A to Right | X or Space, Z or Left Shift, Right Shift or Tab, Enter, the arrows or W A S D | South, West or East, Select, Start, the D-pad (and the left stick) | Yes |
| Controller 2 | M, N, H, Y, I K J L | A second gamepad's | Yes |
| Settings | Esc, F1 | The right stick's press | Yes ([the settings screen](settings.md)) |
| Reset | F5 | | Yes |
| Rewind (held) | Backspace | The left trigger (`LeftTrigger2`) | Yes. Off while Console only is on |
| Pause, one frame, slow motion | F2, F3, F4 | | Yes |
| Scale, widescreen, flash dimming | F8, F9, F10 | | Yes |
| Mute a channel | 5 to 9 | | Yes |
| Save slot N, load slot N | Shift+1 to Shift+4, 1 to 4 | | Yes. Off while Console only is on |
| Quick save, quick load (to the last slot used, slot 1 at first) | | The right bumper (`RightTrigger`), the left bumper (`LeftTrigger`) | Only with Quick save buttons on |
| A shell screen's up, down, left, right, choose, back | The arrows or W A S D; Enter, Space or X; Esc, Backspace or Z | The D-pad, South, East | Yes |
| The pause menu, next mode, hitboxes | P (gamepad Mode), F6, F7 | | No: rows marked `bevy`, the dropped generated game's |

The mechanic page's own keys (F5 and F8 for the original's quick save and load) are a tool's and not in the table.

## The standard's checks

| Rule | Checked by |
|---|---|
| A target passes the shell check: on an input, it saves to slot 1 through its storage a third of the way in, with a stamp, and the Saves line lists it with its frame, mode and stamp; it rewinds through the ring from two thirds back to halfway and autosaves there, listed as Resume; it wanders off with other input, is refused another build's save ("Can't load a save from another version"), another game's, a version 1 save and a damaged one with the game left where it was, and loads slot 1 back from its storage. Then slot 1 is exported, handed back as a file and loaded from it after wandering off again, a damaged file is refused, and Resume loads the autosave. The shell's settings are kept and read back through its storage, off at first, Console only turning off every save, load and rewind binding and the quick save buttons acting only once turned on. It plays on; every frame it lands on has the straight run's hash; then both slots are deleted with their stamps | gp-web's `the_preview_passes_the_shell_check`: Moss Hop in the preview (`Preview` through `ShellTarget`, storage in memory), on 240 frames of made-up input (`made_up_input`) |
| No key or button is on two actions of one target at once | `no_key_or_button_is_on_two_actions_of_a_target` (gp-present) |
| Every button has its place in the standard layout, and the settings keys are the settings screen's | `every_button_has_its_place_in_the_standard_layout`, `the_table_keeps_each_target_s_keys` (gp-present) |
| The preview's saves in a browser, on each ROM: Shift+1 keeps a record in IndexedDB and 1 loads it onto the hash of the frame saved, and held Backspace rewinds; then each feature below played with keys and a stub gamepad | The public page's check, `scripts/web/public/check-page.mjs` (in `just web-page-release`) |

Not here yet: game-printer also ran the shell check in `gp gen conform` (both targets, after the hash comparison), in `gp template check` (each template's model in the interpreter) and on every skeleton `gp game new` writes. Of those, `gp gen` went with the Bevy generator, and gameprinter's `gp template check` checks a ROM against the template only.

| Test | What it checks |
|---|---|
| `gp_present::shell::tests` | The check passing on a made-up game and failing one that loads another build's save or loads a frame late; the ring's step, cap and stepping back; slot names; memory storage; the autosave policy; the key table; a slot's line from its record and stamp (another version's refused, a damaged one, play time and age in words); the shell's settings kept and turning keys off |
| `gp_present::settings::tests` | The Saves line: four slots and Resume once kept, each line's words, choosing a slot and what can be done with it, a load closing the screen with the game's switches kept, Console only hiding the slots, and Cancel putting the shell's settings back |
| gp-web `the_preview_passes_the_shell_check` | Moss Hop in the preview: the check, a slot's line read by the preview, and the module's ring |
| gp-web `the_preview_autosaves_as_a_level_starts` | Moss Hop walking into the first slug: the autosave is due on the frames the level starts again and no others |

## Saves on the settings screen

The seven save features the user approved on 2026-11-04 and 2026-11-05 are on this standard. Each is written once in the shared code and built and tested in the web preview.

| Feature | The shared part | The web preview |
|---|---|---|
| 1. Save slots on the settings screen | A Saves line in `gp_present::settings` (`Screen::with_saves`): Slot 1 to Slot 4, each with its play time, mode and how long ago it was saved ("Slot 2  3:41  Normal  5 min ago"), its level in the line under the list when the game has more than one. Choosing a slot asks Save here, Load, Download as a file, Delete or Back; the host does it (`Outcome::Slot`) and says how it went under the list (`Screen::say`). A load closes the screen with the switches the game runs in (`Screen::close_loaded`), since a restart would undo it | Passes its storage's slots to the module (`gp_settings_slot`, `gp_settings_saves`) as the screen opens |
| 2. Saves from a controller | The right stick's press opens the screen, so feature 1 gives a controller saves. "Quick save buttons" (`Settings::quick_buttons`), off by default, puts quick save on the right bumper and quick load on the left, to the last slot used: two rows of the key table, `needs` `quick-buttons`. Rewind is on the left trigger, and B on East | Reads the rows from shell-keys.js |
| 3. An autosave at each level start | `autosaves` says when; slot 0 is listed as Resume, above the slots, once kept | Writes slot 0 after a tick when `gp_autosave_due` says a level started |
| 4. Browser saves in IndexedDB, with persistence asked | `persist` in the interface | Asks on the player's first save of a visit; moves the slots kept in localStorage into IndexedDB and removes their keys ("Saves kept before the standard") |
| 5. Console only | `Settings::console_only`, a line on the screen beside Theme ("[ ] Console only"); the Saves line says "Off: console only is on". The key table's save, load and rewind rows have `needs` `not-console-only` | Keeps it as `settings/shell.txt` in its storage; refuses those keys and buttons and skips the autosave while it is on |
| 6. The preview's options kept by game | | Scale, widescreen, flash dimming and mute are kept as `gp-view:<model>:<option>` in localStorage. A key of the build's digest (`gp-shell:<digest>:<option>`, before 2026-11-05) is read once, moved and removed |
| 7. Download a save, load one from a file | `export` and `import`; the slot's "Download as a file" (as `<game>-slotN.gpsave`) and "Load from a file" on the Saves line. A file goes through `gp_sim::save` and is refused with the same sentences | Downloads; opens the browser's file chooser |
