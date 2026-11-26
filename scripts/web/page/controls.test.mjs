// node --test scripts/web/page/controls.test.mjs
// GP_CONTROLS_WASM names gp_controls.wasm (scripts/web/build-preview.sh's
// output); without it the test is skipped.
//
// The module against mechanic/input-timeline.js, the rules the owner tuned
// by hand on the screen page: the same changes of the buttons give every
// game frame the same buttons, and the same travel of a lever the same
// pulls. While both exist they must agree.
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { InputTimeline, StickPull } from "../../../mechanic/input-timeline.js";
import { Controls } from "./controls.js";

const wasm = process.env.GP_CONTROLS_WASM;
const skip = wasm ? false : "GP_CONTROLS_WASM isn't set";
const open = (layout, options) => new Controls(new WebAssembly.Instance(new WebAssembly.Module(readFileSync(wasm)), {}).exports, layout, options);

const F = 1000 / 30;
let seed = 7;
const rnd = () => (seed = (seed * 1103515245 + 12345) % 2147483648) / 2147483648;

test("each game frame reads the buttons input-timeline.js reads", { skip }, () => {
  for (const opts of [{ leadMs: 0, assistMs: 0 }, { leadMs: 8, assistMs: 0 }, { leadMs: 8, assistMs: 30 }, { leadMs: 33, assistMs: 5 }]) {
    const c = open("arcade", opts);
    const tl = new InputTimeline(opts);
    // Made-up play: the tap key and the stick's keys going down and up at
    // odd times, browser frames 4 to 70 ms apart.
    let t = 0, now = 0, acc = 0, last = 0, keys = 0;
    for (let n = 0; n < 4000; n++) {
      const until = now + 4 + rnd() * 66;
      while (t < until) {
        t += rnd() * 40;
        if (t >= until) break;
        keys ^= [64, 32, 16, 8][Math.floor(rnd() * 4)];
        tl.set(keys, t);
        c.x.gp_controls_set(keys, t);
      }
      now = until;
      acc = Math.min(acc + (now - last), F * 4);
      last = now;
      while (acc >= F) {
        acc -= F;
        assert.equal(c.x.gp_controls_take(now - acc, now), tl.take(now - acc, now), `frame at ${now - acc} with ${JSON.stringify(opts)}`);
      }
    }
  }
});

test("a lever's pulls are input-timeline.js's", { skip }, () => {
  for (const travel of [1, 6, 20, 50]) {
    const c = open("arcade", { stickTravel: travel });
    const pull = new StickPull(travel);
    // The second stick at rest once, then wandering.
    c.x.gp_controls_pad(0, 0, 0, 0, 0, 0);
    c.x.gp_controls_set(0, 0);
    let m = 0;
    for (let n = 1; n < 5000; n++) {
      m = Math.min(1, Math.max(0, m + (rnd() - 0.5) * 0.3));
      c.x.gp_controls_pad(0, 0, 0, 0, 0, Math.fround(m));
      const bits = c.x.gp_controls_set(0, n);
      assert.equal(bits === 64, pull.read(Math.fround(m)), `step ${n}, the stick at ${m}, travel ${travel}`);
    }
  }
});

test("the layouts' keys", { skip }, () => {
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
