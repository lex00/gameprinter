// The mechanic page's input table, checked without a browser: the block
// between "// <actions>" and "// </actions>" in app.js is run on its own.
// Run with `node mechanic/actions.test.mjs`; `cargo test` runs it too.
import fs from "node:fs";
import assert from "node:assert/strict";

const src = fs.readFileSync(new URL("./app.js", import.meta.url), "utf8");
const block = src.slice(src.indexOf("// <actions>"), src.indexOf("// </actions>"));
const { ACTIONS, NEXT, KEY_INPUT, actionFor, modeKey } = new Function(`${block}; return { ACTIONS, NEXT, KEY_INPUT, actionFor, modeKey };`)();

const INPUTS = [...new Set(Object.values(KEY_INPUT)), "rec", "save", "load"];

/** Presses in one go, as one poll reads them: each looks up the state the
 * one before it left. Returns the actions that ran. */
function press(mode, inputs, typing = false) {
  const ran = [];
  for (const i of inputs) {
    const a = actionFor(mode, i, typing);
    if (!a) continue;
    ran.push(a);
    mode = NEXT[a] ?? mode;
  }
  return { ran, mode };
}

// Sending: nothing does anything.
for (const i of INPUTS) assert.equal(actionFor("sending", i, false), null, `sending: ${i}`);

// Enter then A, and A with Start in one poll: one hand-off.
assert.deepEqual(press("review", ["start", "a"]), { ran: ["handOff"], mode: "sending" });
assert.deepEqual(press("review", ["a", "a", "start", "b", "rec"]), { ran: ["handOff"], mode: "sending" });

// Typing a note: letters are text, Enter hands off and Esc drops.
assert.equal(actionFor("review", "a", true), null);
assert.equal(actionFor("review", "rec", true), null);
assert.equal(actionFor("review", "start", true), "handOff");
assert.equal(actionFor("review", "esc", true), "discard");
// No sealing here: Select does nothing in review.
assert.equal(actionFor("review", "select", false), null);
assert.equal(actionFor("live", "select", false), null);

// C or the left bumper from play shows the newest clip.
assert.equal(actionFor("live", "cmp", false), "compareNewest");
assert.equal(actionFor("agent", "cmp", false), null);

// Answering: A and Start together answer once.
assert.deepEqual(press("agent", ["a", "start"]), { ran: ["answer"], mode: "live" });

// The record button starts a recording, then stops it into review.
assert.deepEqual(press("live", ["rec", "rec"]), { ran: ["startRec", "stopRec"], mode: "review" });

// FREE PLAY has its own rows: Esc goes back to HELP WITH MECHANICS, which
// it does nothing in; saving, loading and recording work in both.
assert.equal(modeKey("live", "free"), "live:free");
assert.equal(modeKey("review", "free"), "review");
assert.equal(modeKey("live", null), "live");
assert.equal(actionFor("live:free", "esc", false), "freeStop");
assert.equal(actionFor("live", "esc", false), null);
assert.equal(actionFor("recording:free", "esc", false), null);
for (const k of ["live", "live:free"]) {
  assert.equal(actionFor(k, "rec", false), "startRec");
  assert.equal(actionFor(k, "save", false), "save");
  assert.equal(actionFor(k, "load", false), "load");
}
for (const k of ["recording", "recording:free"]) assert.equal(actionFor(k, "rec", false), "stopRec");
// No key means two things in one row: each row maps an input to one action.
for (const [m, row] of Object.entries(ACTIONS)) for (const [i, a] of Object.entries(row)) assert.equal(typeof a, "string", `${m}.${i}`);

// Every action that runs on A or Start moves the state, so no action
// repeats.
for (const [mode, map] of Object.entries(ACTIONS)) {
  for (const i of ["a", "start"]) {
    const a = map[i];
    if (a) assert.ok(NEXT[a] && NEXT[a] !== mode, `${mode}: ${i} -> ${a} must leave ${mode}`);
  }
}
console.log("mechanic page actions: ok");
