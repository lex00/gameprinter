// node --test mechanic/input-timeline.test.mjs
import test from "node:test";
import assert from "node:assert/strict";
import { InputTimeline, settings, DEFAULTS } from "./input-timeline.js";

const TAP = 64;
const F = 1000 / 30; // a 30 frames a second game

/** Plays a timeline of [time, bits] changes against browser frames at the
 * times in `draws`: each steps the game frames due, as screen.js's loop
 * does, and gives the bits every game frame read. */
function play(changes, draws, opts = {}) {
  const tl = new InputTimeline({ leadMs: 0, ...opts });
  const read = [];
  let acc = 0, last = 0, i = 0;
  for (const now of draws) {
    while (i < changes.length && changes[i][0] <= now) { tl.set(changes[i][1], changes[i][0]); i++; }
    acc = Math.min(acc + (now - last), F * 4);
    last = now;
    while (acc >= F) {
      acc -= F;
      read.push(tl.take(now - acc, now));
    }
  }
  return read;
}
const frames = (read, bit) => read.filter((b) => b & bit).length;
/** Browser frames every `ms` from 0 to `end`. */
const every = (ms, end) => Array.from({ length: Math.floor(end / ms) + 1 }, (_, k) => k * ms);

test("a late browser frame doesn't stretch a press", () => {
  // A 40 ms pull, then the browser stalls for 4 game frames and catches up.
  const changes = [[100, TAP], [140, 0]];
  const draws = [...every(1000 / 60, 100), 100 + F * 4, 100 + F * 5, 100 + F * 6];
  const read = play(changes, draws);
  // 40 ms is one or two game frames, never the four the catch-up stepped.
  assert.ok(frames(read, TAP) >= 1 && frames(read, TAP) <= 2, `${frames(read, TAP)} frames`);
});

test("a release isn't seen late after a stall", () => {
  // Held 200 ms (6 frames), let go just before a stall of 4 frames.
  const changes = [[100, TAP], [300, 0]];
  const draws = [...every(1000 / 60, 300), 300 + F * 4, 300 + F * 5];
  const n = frames(play(changes, draws), TAP);
  assert.ok(n >= 6 && n <= 7, `${n} frames for a 200 ms pull`);
});

test("a tap shorter than a frame is read once", () => {
  for (const at of [100, 105, 110, 117, 125, 131]) {
    const read = play([[at, TAP], [at + 6, 0]], every(1000 / 60, 400));
    assert.equal(frames(read, TAP), 1, `a 6 ms tap at ${at}`);
  }
});

test("the same pull reads the same within a frame, whatever the browser's phase", () => {
  // 150 ms at 30 a second is 4 or 5 frames by where it starts, as on the
  // machine itself; how the browser's frames fall must not change it.
  for (const start of [100, 108, 116, 124]) {
    const counts = new Set();
    for (const phase of [0, 3, 7, 11, 15]) {
      const draws = every(1000 / 60, 500).map((t) => t + phase);
      counts.add(frames(play([[start, TAP], [start + 150, 0]], draws), TAP));
    }
    assert.equal(counts.size, 1, `a pull at ${start}: ${[...counts]}`);
  }
});

test("assist holds a press on, so a pull a little short counts", () => {
  // 5 frames are asked for (167 ms); the pull is 150.
  const short = [[100, TAP], [250, 0]];
  const draws = every(1000 / 60, 500);
  const bare = frames(play(short, draws), TAP);
  const helped = frames(play(short, draws, { assistMs: 20 }), TAP);
  assert.ok(helped >= 5, `${helped} frames with 20 ms of assist`);
  assert.ok(helped >= bare);
});

test("lead reads ahead of a frame's own time, never past now", () => {
  // Pressed 5 ms after a game frame's time, read by a browser frame 10 ms
  // after it: with no lead the frame doesn't have it, with 8 ms it does.
  const tl0 = new InputTimeline({ leadMs: 0 });
  const tl8 = new InputTimeline({ leadMs: 8 });
  for (const tl of [tl0, tl8]) tl.set(TAP, 105);
  assert.equal(tl0.take(100, 110) & TAP, 0);
  assert.equal(tl8.take(100, 110) & TAP, TAP);
  const early = new InputTimeline({ leadMs: 50 });
  early.set(TAP, 120);
  assert.equal(early.take(100, 110) & TAP, 0, "a press after now isn't read");
});

test("presses are measured: the ms held and the frames that read them", () => {
  const tl = new InputTimeline({ leadMs: 0 });
  tl.set(TAP, 100);
  for (let t = 100; t < 300; t += F) tl.take(t, t);
  tl.set(0, 300);
  tl.take(300, 300);
  const [p] = tl.presses(TAP);
  assert.equal(p.ms, 200);
  assert.equal(p.frames, 6);
});

test("settings stay in range and fall back to the defaults", () => {
  assert.deepEqual(settings(null), { ...DEFAULTS });
  assert.deepEqual(settings('{"leadMs": 900, "assistMs": -3, "pollMs": "2"}'), { leadMs: 50, assistMs: 0, pollMs: 2 });
  assert.deepEqual(settings("not json"), { ...DEFAULTS });
});
