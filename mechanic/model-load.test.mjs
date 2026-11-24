// The wait for the model's preview (mechanic/model-load.js), checked
// without a browser on a made-up clock. Run with
// `node --test mechanic/model-load.test.mjs`; `cargo test` runs it too.
import test from "node:test";
import assert from "node:assert/strict";
import { waitForModel, stoppedWords } from "./model-load.js";

/** A clock the test moves: timers fire when `pass(ms)` passes their time. */
function clock() {
  let now = 0;
  let next = 1;
  const timers = new Map();
  return {
    timer: (f, ms) => { const id = next++; timers.set(id, [now + ms, f]); return id; },
    clear: (id) => timers.delete(id),
    pass(ms) {
      now += ms;
      for (const [id, [at, f]] of [...timers]) if (at <= now) { timers.delete(id); f(); }
    },
  };
}

const outcome = (p) => p.then(() => "ready", (e) => e.message);

test("a load that keeps telling its steps isn't failed however long it takes", async () => {
  const c = clock();
  let started = false;
  const w = waitForModel(() => { started = true; }, c);
  assert.ok(started);
  // 25 s a step, four steps: 100 s in all, never 30 s with no word.
  for (const s of ["script", "instantiated", "board", "theme"]) {
    c.pass(25000);
    w.step(s);
  }
  c.pass(25000);
  w.ready();
  assert.equal(await outcome(w.done), "ready");
});

test("a load that stops says the last step it reached", async () => {
  const c = clock();
  const w = waitForModel(() => {}, c);
  w.step("script");
  c.pass(1000);
  w.step("board");
  c.pass(29999);
  w.step("theme");
  c.pass(30000);
  assert.equal(await outcome(w.done), "the model stopped loading: nothing came for 30 s after the theme was drawn");
  // A late ready changes nothing.
  w.ready();
});

test("a page whose script never ran says so", async () => {
  const c = clock();
  const w = waitForModel(() => {}, c);
  c.pass(30000);
  assert.equal(await outcome(w.done), "the model's page sent nothing for 30 s: its script didn't start");
});

test("the preview's own failure is passed on at once", async () => {
  const c = clock();
  const w = waitForModel(() => {}, c);
  w.step("opened");
  w.failed("board/sprites.bin: this build hasn't got it");
  assert.equal(await outcome(w.done), "its preview didn't start: board/sprites.bin: this build hasn't got it");
});

test("a step the page has no words for is named as it came", () => {
  assert.equal(stoppedWords("new-step", 30000), "the model stopped loading: nothing came for 30 s after new-step");
});
