// The game view's lockstep panel (docs/inspector.md, "The game view"):
// each measure named in the model's words, the one-line summary, and the
// chart's points and each measure's histogram. The game and its numbers are made up.
import { describe, expect, it } from "vitest";
import type { GameWords } from "../src/api";
import { binName, columns, firstFocus, focusMisses, focusWords, histogram, missLine, rangeLine, scaled, lockSummary, measureLine, measureName, mechanicName, missingLine, resyncLine, series, startPhase, driftAt, driftWords, driftThreshold, type Lockstep } from "../src/lockfold";

const g = {
  vocab: {
    entities: [
      { name: "frog", role: "player", description: null },
      { name: "heron", role: "enemy", description: null },
    ],
    levels: [],
    counters: ["flies"],
    rules: {},
  },
  mechanics: { mechanics: [{ name: "platformer.jump", about: "Jumping with the jump action", class: "verb", params: [] }], paths: [] },
} as unknown as GameWords;

const path = (p: string, first: number | null, max = 0) => ({
  path: p,
  level: "exact",
  tolerance: 0,
  within: first === null ? 3 : 2,
  compared: 3,
  mean: 0,
  max,
  first_split: first === null ? null : { frame: first - 4, original_frame: first },
  owners: p.includes("vy") ? ["platformer.jump"] : ["core.counters"],
  distance: first === null ? [0, 0, 0] : [0, 0, max],
});

const ls: Lockstep = {
  mode: "resync",
  measured: 3,
  frames: [
    [0, 346],
    [1, 347],
    [2, 348],
  ],
  paths: [path("counters.flies", null), path("entities.frog[0].vy", 348, 4)],
  splits: [{ at: 2, frame: 2, original_frame: 348, paths: [{ path: "entities.frog[0].vy", original: "-4", model: "0", distance: 4 }] }],
  mechanics: [{ mechanic: "platformer.jump", splits: 1, paths: ["entities.frog[0].vy"], first: { frame: 2, original_frame: 348 } }],
  start: null,
  resync_notes: [
    { path: "entities.heron[*]", how: "missing", why: "" },
    { path: "rng", how: "missing", why: "" },
    { path: "counters.flies", how: "loaded", why: "" },
  ],
};

describe("the lockstep panel", () => {
  it("names each measure in the model's words", () => {
    expect(measureName(g, "entities.frog[0].vy")).toBe("Frog's speed up and down");
    expect(measureName(g, "entities.heron[1].x")).toBe("the second heron's position across");
    expect(measureName(g, "counters.flies")).toBe("the flies");
    expect(measureName(g, "cameras.main.x")).toBe("the view's position across");
    expect(mechanicName(g, "platformer.jump")).toBe("jump (jumping with the jump action)");
  });
  it("says what agrees and what splits first, and when", () => {
    expect(lockSummary(g, ls)).toBe("The two agree on the flies on every frame; Frog's speed up and down splits first, at 5.8 seconds (jump). Put back at every split, they part 1 time.");
    expect(lockSummary(g, { ...ls, mode: "free" })).toMatch(/drift apart\.$/);
    expect(measureLine(ls.paths[1])).toBe("Together on 2 of 3 frames (67%). Farthest apart: 4 px a frame.");
    expect(missingLine(g, ls.resync_notes)).toBe("Not carried over, since the state mapping doesn't hold them yet: the herons and the random numbers.");
  });
  it("says what a put-back couldn't load once: not again when the start's line already says it", () => {
    expect(resyncLine(g, ls)).toBe(missingLine(g, ls.resync_notes));
    const started = { ...ls, start: { frame: 0, original_frame: 38, notes: [ls.resync_notes[1], ls.resync_notes[0]] } };
    expect(resyncLine(g, started)).toBe("");
    expect(resyncLine(g, { ...started, start: { frame: 0, original_frame: 38, notes: [ls.resync_notes[1]] } })).toBe(missingLine(g, ls.resync_notes));
    expect(resyncLine(g, { ...ls, mode: "free" })).toBe("");
    expect(resyncLine(g, { ...ls, splits: [] })).toBe("");
  });
  it("puts the points at the original's frames, and a long run in columns of equal time with each one's average, largest and splits", () => {
    expect(series(ls, 1).map((p) => p.frame)).toEqual([346, 347, 348]);
    const cols = columns(ls, 1, 348, 116);
    expect(cols).toHaveLength(116);
    const last = cols.at(-1)!;
    expect(last).toMatchObject({ from: 346, to: 348, frames: 3, max: 4, splits: 1, first: 348 });
    expect(last.mean).toBeCloseTo(4 / 3);
    expect(cols[0].frames).toBe(0);
    // The doubling scale: a subpixel miss shows, and the largest is the top.
    expect(scaled(0, 300, 1 / 16)).toBe(0);
    expect(scaled(300, 300, 1 / 16)).toBe(1);
    expect(scaled(1 / 16, 300, 1 / 16)).toBeGreaterThan(0.05);
  });

  it("counts each measure's distances up to the view's frame, so the histogram grows as the games play", () => {
    // Frog's vy: 0, 0, then 4 apart; whole numbers, a bin a value, past 0 red.
    const all = histogram(ls, 1);
    expect(all.map((b) => b.frames)).toEqual([2, 0, 0, 0, 1]);
    expect(all.map((b) => b.past)).toEqual([false, true, true, true, true]);
    expect(binName(all[4], ls.paths[1])).toBe("4 px a frame");
    expect(histogram(ls, 1, 347).map((b) => b.frames)).toEqual([2, 0, 0, 0, 0]);
    // Never apart: together on every frame, none apart.
    expect(histogram(ls, 0).map((b) => [b.frames, b.past])).toEqual([
      [3, false],
      [0, true],
    ]);
  });

  it("bins misses past the tolerance in doubling sizes, the first bin holding the frames within it", () => {
    const p = { ...path("entities.frog[0].x", 347, 30.5), tolerance: 2, distance: [0.25, 2, 30.5] };
    const bins = histogram({ ...ls, paths: [p] }, 0);
    expect(bins.map((b) => [binName(b, p), b.frames])).toEqual([
      ["within 2 px", 2],
      ["2 to 4 px", 0],
      ["4 to 8 px", 0],
      ["8 to 16 px", 0],
      ["16 to 32 px", 1],
    ]);
    // Exact: subpixel misses from the smallest one up, as fractions; capped, the last bin holds the rest.
    const q = { ...path("entities.frog[0].x", 347, 300), distance: [0, 0.0625, 0.3, 300] };
    const sub = histogram({ ...ls, frames: [...ls.frames, [3, 349]], paths: [q] }, 0, Infinity, 10, 6);
    expect(sub.map((b) => [binName(b, q), b.frames])).toEqual([
      ["together", 1],
      ["up to 1/16 px", 1],
      ["1/16 to 1/8 px", 0],
      ["1/8 to 1/4 px", 0],
      ["1/4 to 1/2 px", 1],
      ["more than 1/2 px", 1],
    ]);
  });

  it("gives a yes or no measure two bars, together and apart", () => {
    const p = { ...path("entities.frog[0].on_ground", 348, 1), distance: [0, 1, 1] };
    const bins = histogram({ ...ls, paths: [p] }, 0);
    expect(bins.map((b) => [binName(b, p), b.frames])).toEqual([
      ["together", 1],
      ["apart", 2],
    ]);
  });

  it("names the stretch in focus, says what doesn't match there, and starts a second before the first split", () => {
    expect(focusWords({ from: 217, to: 516 })).toBe("5 seconds: 3.6 to 8.6 seconds");
    const f = { from: 347, to: 348 };
    const m = focusMisses(ls, f);
    expect(m.map((x) => [x.path, x.apart, x.compared, x.first])).toEqual([["entities.frog[0].vy", 1, 2, 348]]);
    expect(missLine(g, m[0])).toBe("Frog's speed up and down: apart on 1 of 2 frames, up to 4 px a frame");
    expect(focusMisses(ls, { from: 346, to: 347 })).toEqual([]);
    expect(rangeLine(ls, 1, { from: 346, to: 347 })).toBe("Together on 2 of 2 frames (100%). Never apart.");
    // The first split is at 348; a second before is before the first measured frame, 346.
    expect(firstFocus(ls, 300)).toEqual({ from: 346, to: 645 });
    // Zoomed columns cover only the stretch.
    const cols = columns(ls, 1, 348, 3, 346);
    expect(cols.map((c) => [c.from, c.to, c.frames, c.splits])).toEqual([
      [346, 346, 1, 0],
      [347, 347, 1, 0],
      [348, 348, 1, 1],
    ]);
  });
  it("says where the rebuilt game's start is: loading, started from the original's state with what wasn't carried over, or failed", () => {
    const pg = { ...g, preview: true } as GameWords;
    expect(startPhase(pg, undefined, 1).state).toBe("loading");
    const ready = startPhase(pg, { original_frame: 100, notes: [{ path: "rng", how: "missing", why: "" }, { path: "entities.heron[*]", how: "missing", why: "" }] }, 120);
    expect(ready.state).toBe("ready");
    expect(ready.says).toBe(
      "Started from the original's state at frame 100 (1.7 seconds), and given the same button presses from there; it is never put back. Not carried over, since the state mapping doesn't hold them yet: the random numbers and the herons.",
    );
    expect(startPhase(pg, { original_frame: 195, notes: [] }, 1).says).toContain("begins at frame 195, so until then it stands still");
    expect(startPhase(pg, undefined, 1, "no state").state).toBe("failed");
    expect(startPhase({ ...g, preview: false } as GameWords, undefined, 1).state).toBe("none");
  });
  it("finds where the two drift apart, past 2 px for a position and half a pixel a frame for a speed, and says it", () => {
    const drifting: Lockstep = {
      ...ls,
      frames: [
        [0, 100],
        [1, 101],
        [2, 102],
        [3, 103],
      ],
      paths: [
        { ...path("entities.frog[0].x", 101, 3), distance: [0, 1, 2.5, 3], drift: undefined },
        { ...path("entities.frog[0].vy", 103, 1), distance: [0, 0.25, 0.25, 1] },
      ],
    };
    expect(driftThreshold(drifting.paths[0])).toBe(2);
    expect(driftThreshold(drifting.paths[1])).toBe(0.5);
    const d = driftAt(drifting, 100)!;
    expect(d).toEqual({ frame: 102, path: "entities.frog[0].x", distance: 2.5, threshold: 2 });
    expect(driftWords(g, d)).toBe("They drifted apart here, at 1.7 seconds: Frog's position across, 2.5 px (more than 2 px)");
    expect(driftAt(drifting, 100, 101)).toBeUndefined();
    expect(driftAt({ ...drifting, paths: [{ ...drifting.paths[0], drift: 4 }, drifting.paths[1]] }, 100)?.path).toBe("entities.frog[0].vy");
  });
});

