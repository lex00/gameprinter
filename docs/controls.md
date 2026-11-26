# The controls

How a hand on a keyboard or a gamepad becomes the buttons a game frame reads. One piece of code decides it, `crates/gp-controls`, built as `gp_controls.wasm`. A page that plays a game does the one thing no module can, reading the browser's keys and gamepads, and hands the readings over through `scripts/web/page/controls.js`. The page decides nothing a hand can feel.

## Why

On 2026-11-25 the owner played Tapper 0.1.12 on the public page: "the joystick pour is really slow in the browser.... it was ok in testing locally", then "this should be handled by the game not the browser". The screen page (`mechanic/screen.js`) and the public player (`scripts/web/page/preview.js`) each had their own JavaScript for it, and they differed in 16 places. The ones a hand feels:

| What | The screen page | The public player, before |
|---|---|---|
| When the buttons are read | every change kept with its time, the gamepad polled every 4 ms, each game frame reading at its own time | once for each browser frame; game frames stepped together read the same buttons, and a press and release between two was lost |
| A lever (the second stick) | pulled on the first of its travel out, let go on the first of its way home | held 700 ms for every pull |
| The cabinet's button | any of the four face buttons | the bottom one |
| A late browser frame | catches up 4 game frames at most | 250 ms of them |
| The window loses the keyboard | every key is up | a key could stay down |

Testing on the screen page said nothing about the public page.

## What the module decides

`crates/gp-controls/src/lib.rs`, ported from the screen page's rules, which the owner tuned by hand on 2026-11-22 (`mechanic/input-timeline.js`, gone since the screen page reads its controls here too):

- `Layout`: which button is which. `Nes`: A on the bottom button, B on the left and the right, Select, Start, the d-pad or the left stick. `Arcade`: Select the coin, Start the start, the d-pad or the left stick, any face button or the second stick the button; a cabinet reads one pad.
- A stick is a direction past half its reach (`STICK_ON`).
- `StickPull`: a lever is pulled once it has gone out the travel (6% of its reach by default) from the lowest it has been since the last pull, past a rest zone of 0.12, and let go once it has come back that far from the furthest it got. An axis never seen near the middle (a trigger a pad reports at -1) is no lever.
- `Timeline`: each change of the buttons with its time; `take(t, now)` gives the buttons the game frame at time `t` reads. It may look `lead_ms` past the frame's time (8 by default), never past now. A press that began and ended since the last frame is read once. `assist_ms` holds a press on after it is let go (0 by default). `last_press` is the newest press let go: the ms it was held and the frames that read it, for the screen page's CONTROLS' TIMING box.
- `Options::clamped`: the lead from 0 to 50 ms, the assist from 0 to 100, the lever's travel from 1 to 50%. `controls.js` reads them back as kept (`gp_controls_option`), so a page shows what the module uses.

The keys of each layout are the shell's table (`gp_present::shell::keys`, written to `shell-keys.js`), picked out in `controls.js` (`LAYOUTS`).

The module shares no memory with the page: numbers in, numbers out (`src/abi.rs`).

## What a page does

```js
import { LAYOUTS, openControls } from "./controls.js";
const controls = await openControls("gp_controls.wasm", "arcade");
controls.start();                      // polls the gamepads every pollMs
addEventListener("keydown", (e) => controls.keyDown(e.code, e.timeStamp));
addEventListener("keyup", (e) => controls.keyUp(e.code, e.timeStamp));
addEventListener("blur", () => controls.clearKeys());
// for each game frame, at its own time:
const buttons = controls.take(frameTime, performance.now());
controls.setOptions({ leadMs: 4 });   // a person's setting; controls.opts as kept
controls.lastPress();                  // {ms, frames} of the newest press, or null
```

A button drawn on a page presses the game the same way. The public page's legend rows (docs/web.md, "The page's look") send the game's frame `{gp: "press", code, down}` with a key of the layout, and the player calls `keyDown` or `keyUp` with that code, so a click is read by the same rules as the key it stands for.

A late browser frame catches up four game frames at most; the frame's own time is the present less what is still owed.

## Where it stands

The public player (`preview.js`) and the screen page (`mechanic/screen.js`, the original beside the model) both read their controls this way, since 2026-11-25. The screen page's server serves `controls.js` and `shell-keys.js` from `scripts/web/page` and `gp_controls.wasm` from beside the player (`scripts/web/build-play.sh` builds both, or `$GP_CONTROLS_WASM`). On the NES layout a gamepad on the screen page is now the shell's table, as on the public page (A the bottom button, B the left and the right); the screen page had A on the bottom and the right, B on the left and the top. The screen page's GAME SPEED stays the page's own.

`scripts/web/page/controls.test.mjs`, run by the page's release check on the module as built for the page, plays fixed cases through it: presses read at each frame's own time, a tap between two frames read once, the lead and the assist, a lever let go on the first of its way home, two NES pads as two controllers, the settings kept inside their ranges and the newest press's ms and frames.

The mechanic page (`mechanic/app.js`, the original played live with the agent's requests over it) reads the game's buttons this way too, since 2026-11-25: each game frame takes them at its own time, where before it read the keys and the first two gamepads once for each frame it stepped, with its own copy of the layouts. It opens the controls with the settings the screen page's CONTROLS' TIMING box keeps in the browser (`gp-input`) and has no box of its own. Its own commands (the bumpers and triggers record, compare, save and load; the settings screen's keys) still read the first gamepad's buttons directly, as they are no game's buttons. As on the screen page, an NES gamepad's right button is now B (it was A), and on an arcade layout the right stick taps too.

A game's own filter of its stick (a cabinet's four-way stick) is the game's: the model's hooks apply it, and its profile's switch turns it off (Tapper's `any_way_stick`).
