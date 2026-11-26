// The public page's key legend (keys.js) on the two layouts the player has
// (page/controls.js LAYOUTS, from the shell's table): the rows, the keys
// each shows and the key a click presses.
//
//   node --test scripts/web/public/keys.test.mjs

import assert from "node:assert/strict";
import { test } from "node:test";
import { LAYOUTS } from "../page/controls.js";
import { capOf, capsOf, legendOf, playGroup } from "./keys.js";

const row = (rows, id) => rows.find((r) => r.id === id);

test("an arcade game's legend: coin 5, start 1, the arrows and the button", () => {
  const rows = legendOf(LAYOUTS.arcade);
  assert.deepEqual(rows.map((r) => r.id), ["coin", "start", "move", "button"]);
  assert.deepEqual(row(rows, "coin").keys, ["Digit5"]);
  assert.equal(row(rows, "coin").press, "Digit5");
  assert.equal(row(rows, "coin").say, "Coin, key 5");
  assert.deepEqual(row(rows, "start").keys, ["Digit1"]);
  assert.deepEqual(row(rows, "move").keys, ["ArrowLeft", "ArrowUp", "ArrowDown", "ArrowRight"]);
  assert.equal(row(rows, "move").press, null);
  assert.deepEqual(row(rows, "button").keys, ["KeyX", "Space"]);
  assert.equal(row(rows, "button").label, "Action");
  assert.equal(row(rows, "button").say, "Action, key X or Space");
});

test("a pad game's legend: its own buttons, with no coin", () => {
  const rows = legendOf(LAYOUTS.nes);
  assert.deepEqual(rows.map((r) => r.id), ["start", "select", "move", "a", "b"]);
  assert.deepEqual(row(rows, "start").keys, ["Enter"]);
  assert.deepEqual(row(rows, "select").keys, ["ShiftRight", "Tab"]);
  assert.deepEqual(row(rows, "a").keys, ["KeyX", "Space"]);
  assert.deepEqual(row(rows, "b").keys, ["KeyZ", "ShiftLeft"]);
  assert.equal(row(rows, "b").press, "KeyZ");
});

test("every key a row presses is a key of its layout", () => {
  for (const layout of Object.values(LAYOUTS)) {
    const keys = new Set(layout.keys.flat());
    for (const r of legendOf(layout)) {
      if (r.press !== null) assert.ok(keys.has(r.press), `${r.id} presses ${r.press}`);
      for (const k of r.keys) assert.ok(keys.has(k), `${r.id} shows ${k}`);
    }
  }
});

test("a layout with no keys has no rows", () => {
  assert.deepEqual(legendOf({ keys: [], names: null, taken: new Set() }), []);
  assert.deepEqual(legendOf(null), []);
});

test("key caps: letters and digits bare, arrows drawn, Shift by its side", () => {
  assert.deepEqual(capOf("KeyX"), { text: "X", arrow: null, say: "X" });
  assert.deepEqual(capOf("Digit5"), { text: "5", arrow: null, say: "5" });
  assert.equal(capOf("ArrowLeft").arrow, "left");
  assert.equal(capOf("ArrowLeft").say, "Left arrow");
  assert.equal(capOf("ShiftRight").text, "R Shift");
  assert.equal(capOf("Escape").text, "Esc");
  assert.equal(capOf("F8").text, "F8");
});

test("the player's key spans become caps: Shift+1-4 is two, a list one each", () => {
  assert.deepEqual(capsOf("Shift+1-4").map((c) => c.map((x) => x.text)), [["Shift", "1-4"]]);
  assert.deepEqual(capsOf("X, Space").map((c) => c.map((x) => x.text)), [["X"], ["Space"]]);
  assert.deepEqual(capsOf("2-4").map((c) => c.map((x) => x.text)), [["2-4"]]);
});

test("the play group lists every key of each button", () => {
  const g = playGroup(LAYOUTS.arcade);
  assert.equal(g.name, "Play");
  assert.deepEqual(g.rows.map((r) => r.name), ["Coin", "Start", "Right", "Left", "Down", "Up", "Action"]);
  assert.equal(g.rows.at(-1).keys, "KeyX, Space");
  assert.equal(playGroup(LAYOUTS.nes).rows[0].name, "A");
});
