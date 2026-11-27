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
    now: () => now,
    // Moves to each timer's time in turn, so one a timer sets fires too.
    pass(ms) {
      const to = now + ms;
      for (;;) {
        const due = [...timers].filter(([, [at]]) => at <= to).sort((a, b) => a[1][0] - b[1][0])[0];
        if (!due) break;
        timers.delete(due[0]);
        now = due[1][0];
        due[1][1]();
      }
      now = to;
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

test("a page whose script never ran is loaded again, then says so", async () => {
  const c = clock();
  let starts = 0;
  const w = waitForModel(() => { starts++; }, c);
  c.pass(30000);
  // Silent for 30 s: started again 2 s on, with 30 s more.
  c.pass(2000);
  assert.equal(starts, 2);
  c.pass(30000);
  assert.equal(await outcome(w.done), "the model's page sent nothing for 30 s: its script didn't start");
  assert.equal(starts, 2);
});

test("a frame that loads with no preview running is loaded again until it runs", async () => {
  const c = clock();
  let starts = 0;
  const w = waitForModel(() => { starts++; }, c);
  // gp serve away moving onto a new build: the browser's own error page
  // loads at once, three times over.
  for (let i = 0; i < 3; i++) {
    c.pass(100);
    w.loaded(false);
    // A second load event while it waits starts nothing more.
    w.loaded(false);
    c.pass(2000);
  }
  assert.equal(starts, 4);
  // Back: the preview runs and tells its steps.
  w.loaded(true);
  w.step("script");
  c.pass(1000);
  w.ready();
  assert.equal(await outcome(w.done), "ready");
  assert.equal(starts, 4);
});

test("a frame that never runs its preview gives up after a minute", async () => {
  const c = clock();
  let starts = 0;
  const w = waitForModel(() => { starts++; }, c);
  for (let i = 0; i < 40 && starts < 100; i++) {
    c.pass(10);
    w.loaded(false);
    c.pass(2000);
  }
  assert.equal(await outcome(w.done), "the model's page loaded but its script never ran (tried again for 60 s): preview.js or a file it imports didn't load");
  assert.ok(starts > 20 && starts < 32, `${starts} starts`);
});

test("a load after the preview spoke is no reason to start again", async () => {
  const c = clock();
  let starts = 0;
  const w = waitForModel(() => { starts++; }, c);
  w.step("script");
  w.loaded(false);
  c.pass(5000);
  assert.equal(starts, 1);
  w.ready();
  assert.equal(await outcome(w.done), "ready");
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
