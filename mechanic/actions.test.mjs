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

// Enter then A (the report), and A with Start in one poll: one hand-off.
assert.deepEqual(press("review", ["start", "a"]), { ran: ["handOff"], mode: "sending" });
assert.deepEqual(press("review", ["a", "a", "start", "b", "rec"]), { ran: ["handOff"], mode: "sending" });

// Typing a note: letters are text, Enter hands off and Esc drops.
assert.equal(actionFor("review", "a", true), null);
assert.equal(actionFor("review", "rec", true), null);
assert.equal(actionFor("review", "start", true), "handOff");
assert.equal(actionFor("review", "esc", true), "discard");

// Sealing: Select (Tab on the keyboard, also while typing) seals once, and
// an A in the same poll hands nothing off; the sealed panel closes on A.
assert.deepEqual(press("review", ["select", "a", "start"]), { ran: ["seal"], mode: "sending" });
assert.equal(actionFor("review", "select", true), "seal");
assert.deepEqual(press("sealed", ["a", "start"]), { ran: ["back"], mode: "live" });
assert.equal(actionFor("live", "select", false), null);

// The rebuild: C or the left bumper from play, and in a shown clip.
assert.equal(actionFor("live", "cmp", false), "compareNewest");
assert.equal(actionFor("agent", "cmp", false), "toggleRebuild");

// A run that spans a stage is offered for sealing: up and down choose
// between sealing and handing off; the seal choice doesn't change the state.
assert.equal(actionFor("review", "up", false), "reviewUp");
assert.equal(actionFor("review", "down", true), "reviewDown");
assert.equal(actionFor("review", "left", true), null);

// Answering: A and Start together answer once.
assert.deepEqual(press("agent", ["a", "start"]), { ran: ["answer"], mode: "live" });

// The record button starts a recording, then stops it into review.
assert.deepEqual(press("live", ["rec", "rec"]), { ran: ["startRec", "stopRec"], mode: "review" });

// Reviewing the rebuild: A plays and pauses (a toggle, meant to repeat),
// the arrows step and set the transparency, Select changes the layout.
assert.equal(actionFor("refine", "a", false), "rvPlay");
assert.equal(actionFor("refine", "left", false), "rvBack");
assert.equal(actionFor("refine", "up", false), "rvMore");
assert.equal(actionFor("refine", "select", false), "rvLayout");
assert.equal(actionFor("refine", "rec", false), "rbPlay");
// [ and ] jump to the difference before and after (ON THE MODEL's buttons).
assert.equal(actionFor("refine", "less", false), "rvPrevDiff");
assert.equal(actionFor("refine", "more", false), "rvNextDiff");
// Playing the rebuild has its own rows: in plain play the left bumper
// opens the newest clip and V and Esc do nothing; beside the original they
// change the layout and stop; in FREE PLAY there is no layout to change.
assert.equal(modeKey("live", "beside"), "live:beside");
assert.equal(modeKey("review", "beside"), "review");
assert.equal(actionFor("live", "cmp", false), "compareNewest");
assert.equal(actionFor("live", "lay", false), null);
assert.equal(actionFor("live", "esc", false), null);
assert.equal(actionFor("live:beside", "cmp", false), "rbLayout");
assert.equal(actionFor("live:beside", "esc", false), "rbStop");
assert.equal(actionFor("recording:beside", "esc", false), null);
assert.equal(actionFor("live:free", "lay", false), null);
assert.equal(actionFor("live:free", "esc", false), "rbStop");
assert.equal(actionFor("live:free", "load", false), null);
// Recording works the same in all three.
for (const k of ["live", "live:beside", "live:free"]) assert.equal(actionFor(k, "rec", false), "startRec");
for (const k of ["recording", "recording:beside", "recording:free"]) assert.equal(actionFor(k, "rec", false), "stopRec");
// No key means two things in one row: each row maps an input to one action.
for (const [m, row] of Object.entries(ACTIONS)) for (const [i, a] of Object.entries(row)) assert.equal(typeof a, "string", `${m}.${i}`);

// Every other action that runs on A or Start moves the state, so no action
// repeats.
for (const [mode, map] of Object.entries(ACTIONS)) {
  if (mode === "refine") continue;
  for (const i of ["a", "start"]) {
    const a = map[i];
    if (a) assert.ok(NEXT[a] && NEXT[a] !== mode, `${mode}: ${i} -> ${a} must leave ${mode}`);
  }
}
console.log("mechanic page actions: ok");
