// The curve chart's pure side (src/curvefold.ts): which measure it shows,
// the short list, and the line being tried.
import { describe, expect, it } from "vitest";
import { type ClipCurves, defaultMeasure, points, sameEntity, tryingLine } from "../src/curvefold";

const curves: ClipCurves = {
  clip: "c1",
  model: "m",
  offset: 1,
  frames: 3,
  tracks: [
    { track: "mario", frame: 1, path: "entities.mario[0].vx", says: "Mario: wrong from frame 1" },
    { track: "camera", frame: 2, path: "cameras.main.x", says: "camera: wrong from frame 2" },
  ],
  drift: { frame: 3, path: "cameras.main.x" },
  measures: [
    { path: "entities.mario[0].x", words: "Mario's place across", track: "mario", original: [1, 2, 3], model: [1, 2, 4] },
    { path: "entities.mario[0].vx", words: "Mario's speed across", track: "mario", original: [0.0625, 0.125, 0.125], model: [0.037, 0.074, 0.111], first_wrong: 1 },
    { path: "entities.mario[0].y", words: "Mario's height", track: "mario", original: [176, 176, 176], model: [176, 176, 176] },
    { path: "cameras.main.x", words: "the camera's place across", track: "camera", original: [0, 0, 1], model: [0, 0, 0], first_wrong: 3 },
  ],
};

describe("the curve chart", () => {
  it("shows the measure the work is on, else the first wrong track's", () => {
    expect(defaultMeasure(curves, null)).toBe("entities.mario[0].vx");
    const job = { clip: "c1", target: { measures: [{ path: "entities.mario[0].x" }] } };
    expect(defaultMeasure(curves, job)).toBe("entities.mario[0].x");
    // Work on another clip doesn't move this one's chart.
    expect(defaultMeasure(curves, { ...job, clip: "c2" })).toBe("entities.mario[0].vx");
  });

  it("lists the same entity's measures, speeds first", () => {
    expect(sameEntity(curves, "entities.mario[0].x").map((m) => m.path)).toEqual(["entities.mario[0].vx", "entities.mario[0].y", "entities.mario[0].x"]);
    expect(sameEntity(curves, "cameras.main.x").map((m) => m.path)).toEqual(["cameras.main.x"]);
  });

  it("puts values at the clip's frames and draws the value being tried", () => {
    expect(points([1, null, 3], 1)).toEqual([
      [1, 1],
      [3, 3],
    ]);
    const job = { clip: "c1", trying: { param: "entities.mario.run.acceleration", words: "acceleration", value: "0.0625", pass: "impact", curve: { path: "entities.mario[0].vx", frames: [1, 2], values: [0.0625, 0.125] } } };
    const t = tryingLine(job, "c1", "entities.mario[0].vx");
    expect(t?.points).toEqual([
      [1, 0.0625],
      [2, 0.125],
    ]);
    expect(t?.says).toBe("trying acceleration = 0.0625 (finding which numbers matter)");
    expect(tryingLine(job, "c1", "entities.mario[0].x")).toBeUndefined();
  });
});

import { trackWords } from "../src/Unified";

describe("a clip's score by track", () => {
  it("names the bodies first and the camera apart", () => {
    const s = trackWords(undefined, [
      { track: "mario", frame: 1, path: "entities.mario[0].vx" },
      { track: "camera", frame: 40, path: "cameras.main.x" },
    ]);
    expect(s.startsWith("Mario: wrong from frame 1 (")).toBe(true);
    expect(s).toContain("; the camera: wrong from frame 40 (");
  });
});
