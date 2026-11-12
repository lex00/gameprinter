// The game view's "Refining" section in words (docs/inspector.md, "The
// game view"): each round of the lockstep refinement loop as a sentence,
// what it's trying, its state, and a stuck window's measures. The game and
// its numbers are made up.
import { describe, expect, it } from "vitest";
import type { GameWords } from "../src/api";
import { measureWords, paramName, refineStatus, roundLine, short, tryingLine, type RefineRound, type RefineState } from "../src/refinefold";

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
  mechanics: { mechanics: [], paths: [] },
} as unknown as GameWords;

const w = (from: number, to: number) => ({ from, to, original_from: from + 200, original_to: to + 200, seconds_from: from / 60, seconds_to: to / 60 });

const round: RefineRound = {
  round: 1,
  window: w(1, 10),
  mechanic: "platformer.run",
  param: "entities.frog.run.run_acceleration",
  unit: "px/frame^2",
  old: "0.037109375",
  new: "0.0625",
  window_splits: { before: 20, after: 0 },
  exact: true,
  first_split: { before: 1, after: 31 },
  next_window: 31,
  together: { before: 600, after: 630 },
};

describe("the refining section", () => {
  it("names the numbers it changes in the model's words", () => {
    expect(paramName(g, "entities.frog.run.run_acceleration")).toBe("Frog's running speed-up");
    expect(paramName(g, "entities.frog.jump.gravity")).toBe("Frog's jump pull down");
    expect(paramName(g, "entities.frog.jump.speed")).toBe("Frog's jump speed");
    expect(paramName(g, "entities.frog.jump.bands[1].gravity_held")).toBe("Frog's jump pull down while jump is held (band 2)");
    expect(paramName(g, "entities.heron.walk.speed")).toBe("the heron's speed");
    expect(short("0.037109375")).toBe("0.037");
    expect(short("0.0625")).toBe("0.0625");
  });
  it("says each round, its window and where the next one starts", () => {
    expect(roundLine(g, round)).toBe("Frames 1 to 10 (0.0 to 0.2 s): Frog's running speed-up 0.037 -> 0.0625 a frame. Now exact; next window starts at frame 31.");
    expect(roundLine(g, { ...round, exact: false, next_window: null, window_splits: { before: 20, after: 11 } })).toMatch(/Closer: 20 splits in the window before, 11 now\.$/);
    expect(roundLine(g, { ...round, next_window: null })).toMatch(/Now exact, and nothing splits after it\.$/);
    expect(tryingLine(g, { round: 2, mechanic: "platformer.run", param: "entities.frog.run.run_acceleration", unit: "px/frame^2", value: "0.05" })).toBe("trying: Frog's running speed-up = 0.05");
  });
  it("says what the loop is doing", () => {
    const base: RefineState = { recording: "r", state: "running", running: true, rounds: [round], current: { round: 3, window: w(31, 40) } };
    expect(refineStatus(base)).toBe("Refining: frames 31 to 40, round 3");
    const q = { id: "q", text: "", window: w(31, 40), mechanic: "platformer.run", measures: [], proposal: "p" };
    const result = { stop: "", questions: [q], first_split: { after: 31 }, together: { before: 1, after: 2, measured: 101 } };
    expect(refineStatus({ ...base, state: "stuck", running: false, current: undefined, result })).toBe("Stuck at frames 31 to 40: a question for you");
    expect(refineStatus({ ...base, state: "done", running: false, result: { ...result, questions: [], first_split: { after: null } } })).toBe("Done: exact to frame 100");
  });
  it("puts a stuck measure's two games side by side", () => {
    const m = { path: "entities.frog[0].x", mechanic: "platformer.run", original: ["40.1875", "40.078125"], model: ["40.0625", "40.3125"], why: "", proposal: "" };
    expect(measureWords(g, m)).toBe("Frog's position across: the original 40.1875, 40.078; the model 40.0625, 40.3125.");
  });
});
