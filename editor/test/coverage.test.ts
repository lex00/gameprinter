// The coverage view's arithmetic: executed bytes per source line and per
// PRG ROM page, and the program counter's line.
import { describe, expect, it } from "vitest";
import { lineCounts, lineState, offsetOfPc, pageRows, toBytes } from "../src/views/executed";

describe("coverage", () => {
  const executed = toBytes({ len: 2048, ranges: [[0, 3], [1030, 1032]] });
  it("expands a set's runs", () => {
    expect(executed.length).toBe(2048);
    expect([...executed.slice(0, 4)]).toEqual([1, 1, 1, 0]);
    expect(executed.reduce((a, b) => a + b, 0)).toBe(5);
  });
  it("counts each line's executed bytes and leaves out spans past PRG ROM", () => {
    const listing = {
      files: [{ name: "game.s", lines: ["reset:", "  sei", "  lda #$00", "  jmp reset", "chr"] }],
      spans: [
        [0, 2, 0, 1],
        [0, 3, 1, 2],
        [0, 4, 3, 3],
        [0, 5, 4096, 16],
      ] as [number, number, number, number][],
    };
    const counts = lineCounts(listing, executed)[0];
    expect(counts.get(2)).toEqual({ bytes: 1, executed: 1, offset: 0 });
    expect(lineState(counts.get(3)!)).toBe("all");
    expect(lineState(counts.get(4)!)).toBe("none");
    expect(counts.has(5)).toBe(false);
    expect(counts.has(1)).toBe(false);
    expect(lineState({ bytes: 3, executed: 1, offset: 0 })).toBe("some");
  });
  it("finds the program counter's offset by the pages coverage saw", () => {
    const windows = [[0xc000], [0xc400]];
    expect(offsetOfPc(0xc402, windows)).toBe(1026);
    expect(offsetOfPc(0x8000, windows)).toBeUndefined();
    expect(offsetOfPc(0x8002, [[0x8000], [0x8000]])).toBeUndefined();
  });
  it("rows pages by bank, in 64-byte cells", () => {
    const rows = pageRows(executed, [[0xc000], [0xc400]], 1024);
    expect(rows.length).toBe(2);
    expect(rows[0].cells.length).toBe(16);
    expect(rows[0].cells[0]).toBe(3);
    expect(rows[1]).toMatchObject({ page: 1, bank: 1, at: [0xc400], executed: 2 });
  });
});
