import { describe, expect, it } from "vitest";
import type { BgState } from "../src/backfold";
import { type GridCell, type GridState, cellClick, cellLine, cellLook, gridSays, shownCols, shownRows, worse } from "../src/gridfold";

const cell = (over: Partial<GridCell> = {}): GridCell => ({ row: "mechanic:platformer.jump", col: "w1_1/start", state: "empty", clips: [], doubt: 0, ...over });
const bg = (over: Partial<BgState> = {}): BgState => ({ enabled: true, scores: {}, questions: [], ...over });

describe("the grid's cells", () => {
  it("take the server's state, and the next pick shows on an empty cell", () => {
    expect(cellLook(cell({ state: "exact", clips: ["a"] }), bg(), undefined)).toBe("exact");
    expect(cellLook(cell({ state: "no_start" }), bg(), undefined)).toBe("no-start");
    expect(cellLook(cell(), bg(), undefined)).toBe("empty");
    const next = { row: "mechanic:platformer.jump", col: "w1_1/start", why: "empty", says: "", clip: "c" };
    expect(cellLook(cell(), bg(), undefined, next)).toBe("next");
    expect(cellLook(cell({ col: "w1_2/start" }), bg(), undefined, next)).toBe("empty");
  });

  it("need you when a clip has a conflict question, amber over everything else", () => {
    const q = { id: "q1", clip: "a", clip_name: "A", at: 3, ask: "check the mapping?", choices: ["check the mapping", "the code is wrong here"], detail: "", model: "m", answer: null, kind: "conflict" };
    const b = bg({ questions: [q] as BgState["questions"] });
    expect(cellLook(cell({ state: "differs", clips: ["a", "b"] }), b, undefined)).toBe("needs-you");
    // The loop's generic shape question on a clip the tool made is not for you.
    const shape = { ...q, kind: "shape", choices: ["too high", "too low", "looks right"], clip: "gen.x" };
    expect(cellLook(cell({ state: "differs", clips: ["gen.x"] }), bg({ questions: [shape] as BgState["questions"] }), undefined)).toBe("differs");
  });

  it("are being fixed while a job or an agent is on one of their clips", () => {
    const b = bg({ jobs: [{ kind: "refine", clip: "b" }] as BgState["jobs"] });
    expect(cellLook(cell({ state: "differs", clips: ["a", "b"] }), b, undefined)).toBe("fixing");
    const a = bg({ clip_agents: [{ clip: "x", clips: ["a"], state: "working" }] as BgState["clip_agents"] });
    expect(cellLook(cell({ state: "close", clips: ["a"] }), a, undefined)).toBe("fixing");
  });

  it("rank worst first", () => {
    expect(worse("exact", "differs")).toBe("differs");
    expect(worse("needs-you", "fixing")).toBe("needs-you");
    expect(worse("empty", "waiting")).toBe("waiting");
  });

  it("say one line, and a click opens the clip in flight first or says it is empty", () => {
    const row = { id: "mechanic:platformer.jump", kind: "mechanic" as const, label: "jump", group: "platformer.jump" };
    const col = { id: "w1_1/start", level: "w1_1", level_name: "World 1-1", place: "start", starts: ["s"] };
    expect(cellLine(row, col, cell({ clips: ["a", "b"] }), "differs")).toBe("jump, World 1-1 start: differs (2 clips)");
    expect(cellLine(row, col, cell(), "next")).toBe("jump, World 1-1 start: empty: next up");
    expect(cellClick(cell({ clips: ["a", "b"] }), "differs", ["b"])).toEqual({ open: "b" });
    expect(cellClick(cell({ clips: ["a", "b"] }), "differs")).toEqual({ open: "a" });
    expect(cellClick(cell(), "empty")).toEqual({ says: "empty: next up" });
    expect(cellClick(cell({ state: "no_start" }), "no-start")).toEqual({ says: "no start here the rebuilt game can begin at" });
  });

  it("leave out the places with no start unless asked, and the rows with no cell there", () => {
    const g: GridState = {
      run: "r",
      mode: "grid",
      in_flight: 3,
      max_in_flight: 12,
      made: 2,
      left: 1,
      rows: [
        { id: "a", kind: "mechanic", label: "a", group: "a" },
        { id: "b", kind: "entity", label: "b", group: "b" },
      ],
      cols: [
        { id: "w1/start", level: "w1", level_name: "W1", place: "start", starts: ["s1"] },
        { id: "w1/pipe", level: "w1", level_name: "W1", place: "pipe", starts: [] },
      ],
      cells: [
        { row: "a", col: "w1/start", state: "exact", clips: ["c"], doubt: 0 },
        { row: "a", col: "w1/pipe", state: "no_start", clips: [], doubt: 0 },
        { row: "b", col: "w1/pipe", state: "no_start", clips: [], doubt: 0 },
      ],
      filled: 1,
      cell_count: 1200,
      flight: [{ id: "c", row: "a", col: "w1/start", why: "empty", says: "" }],
    };
    const cols = shownCols(g, false);
    expect(cols.map((c) => c.id)).toEqual(["w1/start"]);
    expect(shownRows(g, cols).map((r) => r.id)).toEqual(["a"]);
    expect(shownCols(g, true).length).toBe(2);
    expect(shownRows(g, shownCols(g, true)).map((r) => r.id)).toEqual(["a", "b"]);
    expect(gridSays(g)).toBe("1 of 1,200 cells filled; 1 in flight, at most 3.");
  });
});
