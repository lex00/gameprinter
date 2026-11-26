// node --test scripts/web/page/controls.test.mjs
// GP_CONTROLS_WASM names gp_controls.wasm (scripts/web/build-preview.sh's
// or build-play.sh's output); without it the test is skipped.
//
// Fixed cases through controls.js and the module as built, the one copy of
// the rules every page that plays a game reads its controls by
// (docs/controls.md): the rules the owner tuned by hand on the screen page
// on 2026-11-22, and the screen page's own measure of the last press.
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { Controls } from "./controls.js";

const wasm = process.env.GP_CONTROLS_WASM;
const skip = wasm ? false : "GP_CONTROLS_WASM isn't set";
const open = (layout, options) => new Controls(new WebAssembly.Instance(new WebAssembly.Module(readFileSync(wasm)), {}).exports, layout, options);

/** The gamepads the browser would give: each `{buttons, axes}`. */
let pads = [];
Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  value: {
    getGamepads: () => pads.map((p, index) => ({
      index,
      connected: true,
      buttons: Array.from({ length: 17 }, (_, i) => ({ pressed: (p.buttons ?? []).includes(i) })),
      axes: p.axes ?? [0, 0, 0, 0],
    })),
  },
});

test("each game frame reads the buttons at its own time", { skip }, () => {
  pads = [];
  const c = open("arcade", { leadMs: 0 });
  // Down at 10 ms, up at 40: frames at 0, 33 and 66, all taken late.
  c.x.gp_controls_set(64, 10);
  c.x.gp_controls_set(0, 40);
  assert.equal(c.x.gp_controls_take(0, 100), 0);
  assert.equal(c.x.gp_controls_take(33, 100), 64);
  assert.equal(c.x.gp_controls_take(66, 100), 0);
  // A tap wholly between two frames is read once.
  c.x.gp_controls_set(64, 70);
  c.x.gp_controls_set(0, 75);
  assert.equal(c.x.gp_controls_take(99, 100), 64);
  assert.equal(c.x.gp_controls_take(132, 140), 0);
});

test("the lead reads ahead, never past now; the assist holds a press on", { skip }, () => {
  pads = [];
  let c = open("arcade", { leadMs: 8 });
  c.x.gp_controls_set(1, 5);
  assert.equal(c.x.gp_controls_take(0, 3), 0, "now is before the press");
  c = open("arcade", { leadMs: 8 });
  c.x.gp_controls_set(1, 5);
  assert.equal(c.x.gp_controls_take(0, 20), 1, "inside the lead");
  c = open("arcade", { leadMs: 0, assistMs: 20 });
  c.x.gp_controls_set(1, 0);
  c.x.gp_controls_set(0, 10);
  assert.equal(c.x.gp_controls_take(25, 25), 1, "let go 15 ms before");
  assert.equal(c.x.gp_controls_take(31, 31), 0);
});

test("a lever is let go on the first of its way home", { skip }, () => {
  pads = [{ axes: [0, 0, 0, 0] }];
  const c = open("arcade", { stickTravel: 6 });
  assert.equal(c.now(), 0, "seen at rest");
  pads = [{ axes: [0, 0, 0, -0.9] }];
  assert.equal(c.now(), 64, "pulled up: the button");
  pads = [{ axes: [0, 0, 0, -0.8] }];
  assert.equal(c.now(), 0, "its travel back from the furthest");
  pads = [{ axes: [0, 0, -0.9, 0] }];
  assert.equal(c.now(), 64, "pulled left, from where it stopped");
  pads = [{ buttons: [3] }];
  assert.equal(c.now(), 64, "any face button is the button");
  assert.equal(c.padBits(), 64, "the pad's alone");
  // A trigger reported as an axis resting at -1 is no lever.
  pads = [];
  const t = open("arcade");
  pads = [{ axes: [0, 0, -1, NaN] }];
  assert.equal(t.now(), 0);
});

test("two NES pads are two controllers", { skip }, () => {
  pads = [{ buttons: [0, 9], axes: [0.9, 0, 0, 0] }, { buttons: [2], axes: [0, -0.9, 0.9, 0.9] }];
  const c = open("nes");
  const bits = c.now();
  assert.equal(bits & 0xff, 1 | 8 | 128, "A, Start, right");
  assert.equal(bits >> 8, 2 | 16, "B, up: a second stick is nothing here");
  pads = [{ buttons: [1] }];
  assert.equal(c.now(), 2, "the right button is B");
  // The second controller's keys too.
  pads = [];
  c.keyDown("KeyM", 1);
  assert.equal(c.bits, 1 << 8);
  c.clearKeys(2);
  // A cabinet reads one pad.
  pads = [{}, { buttons: [0] }];
  assert.equal(open("arcade").now(), 0);
});

test("the layouts' keys", { skip }, () => {
  pads = [];
  const c = open("arcade");
  c.keyDown("Digit5", 1);
  assert.equal(c.bits, 1, "5 drops a coin");
  c.keyUp("Digit5", 2);
  c.keyDown("Space", 3);
  c.keyDown("ArrowUp", 3);
  assert.equal(c.bits, 64 | 32);
  c.clearKeys(4);
  assert.equal(c.bits, 0);
  c.setLayout("nes");
  c.keyDown("KeyX", 5);
  c.keyDown("ArrowRight", 5);
  assert.equal(c.bits, 1 | 128);
});

test("the settings are kept inside their ranges", { skip }, () => {
  pads = [];
  const c = open("arcade", { leadMs: 500, assistMs: -3, stickTravel: 0, pollMs: 99 });
  assert.deepEqual(c.opts, { leadMs: 50, assistMs: 0, stickTravel: 1, pollMs: 16 });
  c.setOptions({ leadMs: "4", assistMs: "not a number" });
  assert.deepEqual(c.opts, { leadMs: 4, assistMs: 0, stickTravel: 1, pollMs: 16 }, "one not a number stays");
});

test("the newest press: the ms held and the frames that read it", { skip }, () => {
  pads = [];
  const c = open("arcade", { leadMs: 0 });
  assert.equal(c.lastPress(), null);
  c.x.gp_controls_set(64, 10);
  c.x.gp_controls_take(0, 5);
  c.x.gp_controls_take(33, 40);
  c.x.gp_controls_take(66, 70);
  c.x.gp_controls_set(0, 80.4);
  assert.deepEqual(c.lastPress(), { ms: 70, frames: 2 });
  // A tap let go before the next frame reads it: that frame counts it.
  c.x.gp_controls_set(1, 105);
  c.x.gp_controls_set(0, 110);
  assert.deepEqual(c.lastPress(), { ms: 5, frames: 0 });
  c.x.gp_controls_take(132, 133);
  assert.deepEqual(c.lastPress(), { ms: 5, frames: 1 });
});
