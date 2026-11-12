// The coverage view's listing and bank grid render from the endpoints' JSON.
import { describe, expect, it } from "vitest";
import { renderToString } from "react-dom/server";
import { Banks, Listing } from "../src/views/Coverage";
import type { CoverageJson } from "../src/views/executed";

const cov: CoverageJson = {
  sha256: "0".repeat(64),
  mapper: 0,
  frames: 10,
  executed: { len: 2048, ranges: [[0, 3]] },
  starts: { len: 2048, ranges: [[0, 1]] },
  windows: [[0xc000], []],
  ram_written: { len: 2048, ranges: [] },
  screens: {},
  levels: {},
};

describe("coverage view", () => {
  it("marks lines that ran and the program counter's line", () => {
    const listing = {
      files: [{ name: "game.s", lines: ["reset:", "  sei", "  lda #$00", "  jmp reset"] }],
      spans: [
        [0, 2, 0, 1],
        [0, 3, 1, 2],
        [0, 4, 3, 3],
      ] as [number, number, number, number][],
    };
    const html = renderToString(<Listing listing={listing} cov={cov} pc={0xc001} />);
    expect(html).toContain("run-all");
    expect(html).toContain("run-none");
    expect(html).toContain("at-pc");
    expect(html).toContain("go to PC $c001");
  });
  it("draws PRG ROM by page without a listing", () => {
    const html = renderToString(<Banks cov={cov} bankSize={16384} />);
    expect(html).toContain("bank 0 +0000 at $c000");
    expect(html).toContain("bank 0 +0400");
  });
});
