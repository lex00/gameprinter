// The curve chart's pure side (src/curvefold.ts): values at the clip's frames.
import { describe, expect, it } from "vitest";
import { points } from "../src/curvefold";

describe("the curve chart", () => {
  it("puts values at the clip's frames", () => {
    expect(points([1, null, 3], 1)).toEqual([
      [1, 1],
      [3, 3],
    ]);
  });
});
