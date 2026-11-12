// The game view's Scenarios (docs/inspector.md, "Scenarios"): the ladder by
// tier with each sequence's mark, what differs, its score, what it needs,
// and the one next step; the refine piece with what the loop is doing; and
// the client's endpoints. The game and its numbers are made up.
import { describe, expect, it } from "vitest";
import { renderToString } from "react-dom/server";
import { RefineOnScenario, ScenarioLadder, ScenarioSideBySide } from "../src/Scenarios";
import { type Ladder, type LadderRow, type ScenariosState, ScenarioClient, byTier, mark, nextAction, nextRow, rowMark, tellingFrame, tierName } from "../src/scenariofold";

const row = (id: string, tier: number, state: LadderRow["state"], first?: number): LadderRow => ({
  scenario: { id, name: `${id} name`, tier, mechanic: "platformer.jump", params: [], frames: 20, presses: ["a@0-4"], requires: tier > 1 ? ["t1.hop.tap"] : [], source: { kind: "generated" } },
  state,
  result: {
    id,
    exact: first === undefined,
    score: first === undefined ? 0 : 3,
    measured: 21,
    original_from: 300,
    original_to: 320,
    first: first === undefined ? undefined : { frame: first, original_frame: 300 + first, path: "entities.frog[0].y", original: "160", model: "168", distance: "8", mechanic: "platformer.jump", paths: ["entities.frog[0].y"] },
    says: "",
    measures: [{ path: "entities.frog[0].y", together: first === undefined ? 21 : 18, total: 21, tolerance: "0", max: first === undefined ? "0" : "8", mean: "1", first: first === undefined ? undefined : { frame: first, original: "160", model: "168" }, bins: [{ from: "0", to: "0", frames: 18 }, { from: "4", to: "8", frames: 3 }], owners: ["platformer.jump"], distance: [] }],
    headline: { exact: first === undefined ? 1 : 0, measures: 1, together_permille: first === undefined ? 1000 : 857, says: first === undefined ? "1/1 measures, 100%" : "0/1 measures, 85%" },
    moved: ["platformer.jump"],
  },
  says: first === undefined ? `${id} name: matches exactly` : `${id} name: the rebuilt Frog goes 8 px lower, from frame ${first} (jump)`,
  needs: state === "locked" ? "needs: t1.hop.tap name" : "",
  unlocks: "",
  history: [],
  trend: first === undefined ? "was 85% -> now 100%" : null,
  video: { input: "walk", from: 300, to: 320 },
});

const ladder: Ladder = {
  state: "done",
  says: "First mismatch: t1.hop.tap name. The rebuilt Frog goes 8 px lower, from frame 9 (jump). Next: fix the jump's numbers.",
  bar_says: "exact on every measure",
  start: { input: "walk", asked: null, original_frame: 300, frame: 296, seconds: 5, says: "Frog standing on the ground at 5.0 s, nothing near", loaded: 9, approximated: 0, missing: 2 },
  next: "t1.hop.tap",
  start_here: "t1.hop.tap",
  first_failing: "t1.hop.tap",
  tiers: [
    { tier: 1, exact: 1, passed: 1, total: 2 },
    { tier: 2, exact: 0, passed: 0, total: 1 },
  ],
  passed: 1,
  locked: 1,
  scenarios: [row("t1.nothing", 1, "passed"), row("t1.hop.tap", 1, "failing", 9), row("t2.hop+run", 2, "locked", 3)],
  model: "abc",
};

describe("the scenarios section", () => {
  it("says each sequence's mark and groups them by tier", () => {
    expect(mark("passed")).toBe("matches");
    expect(mark("failing")).toBe("doesn't match");
    expect(tierName(1)).toBe("Each button alone");
    expect(tierName(0)).toBe("Played by you");
    const t = byTier(ladder);
    expect(t.map((x) => [x.tier, x.rows.length, x.exact, x.total])).toEqual([
      [1, 2, 1, 2],
      [2, 1, 0, 1],
    ]);
    expect(nextRow(ladder)?.scenario.id).toBe("t1.hop.tap");
    // A thumbnail shows the first split, else near the end.
    expect(tellingFrame(ladder.scenarios[1])).toBe(309);
    expect(tellingFrame(ladder.scenarios[0])).toBe(300);
  });

  it("marks the sequence a job is working on, and names the one next step", () => {
    const st: ScenariosState = { recording: "walk", state: "running", job: "refine", says: "Trying gravity", refine: { scenario: "t1.hop.tap", rounds: [], trying: { param: "entities.frog.jump.gravity", value: "0.25", mechanic: "platformer.jump" } } };
    expect(rowMark(ladder.scenarios[1], st)).toBe("being worked on");
    expect(rowMark(ladder.scenarios[0], st)).toBe("matches");
    expect(nextAction(undefined)).toBe("run");
    expect(nextAction(st)).toBe("wait");
    expect(nextAction({ recording: "walk", state: "done", says: "", ladder })).toBe("refine");
    expect(nextAction({ recording: "walk", state: "done", says: "", ladder: { ...ladder, next: null } })).toBe("record");
  });

  it("draws the ladder, one sequence's scores and the refine piece from props", () => {
    const html = renderToString(<ScenarioLadder ladder={ladder} selected="t1.hop.tap" />);
    expect(html).toContain("Each button alone: 1 of 2 match");
    expect(html).toContain("the rebuilt Frog goes 8 px lower, from frame 9 (jump)");
    expect(html).toContain("start here");
    expect(html).toContain("needs: t1.hop.tap name");
    expect(html).toContain("was 85% -&gt; now 100%");
    expect(html).toContain("0/1 measures, 85%");
    const side = renderToString(<ScenarioSideBySide row={ladder.scenarios[1]} />);
    expect(side).toContain("18<!-- --> of <!-- -->21");
    expect(side).toContain("frame 9: 168 for 160");
    const st: ScenariosState = { recording: "walk", state: "done", says: "", refine: { scenario: "t1.hop.tap", rounds: [{ round: 1, param: "entities.frog.jump.gravity", old: "0.1875", new: "0.25" }], trying: null, result: { scenario: "t1.hop.tap", name: "", says: "t1.hop.tap name now passes; also fixes 3 other scenarios, none worse.", passes: true, changed: true, also_fixed: [], worse: [] } } };
    const r = renderToString(<RefineOnScenario row={ladder.scenarios[1]} state={st} start={() => undefined} />);
    expect(r).toContain("Refine on this sequence");
    expect(r).toContain("0.1875 -&gt; 0.25");
    expect(r).toContain("none worse");
  });

  it("asks the run's scenarios endpoints", async () => {
    const asked: [string, RequestInit | undefined][] = [];
    const f = (async (u: string, init?: RequestInit) => {
      asked.push([u, init]);
      return new Response(JSON.stringify({ started: true, recording: "walk", state: "idle", says: "" }), { status: 200 });
    }) as unknown as typeof fetch;
    const c = new ScenarioClient("run 1", "", f);
    await c.state("walk");
    await c.refine("walk", "t1.hop.tap");
    await c.played("walk", 300, "gp-input-log 0\n");
    expect(asked[0][0]).toBe("/api/capture/run%201/scenarios?recording=walk");
    expect(asked[1][0]).toBe("/api/capture/run%201/scenarios/refine");
    expect(JSON.parse(asked[1][1]!.body as string)).toEqual({ recording: "walk", scenario: "t1.hop.tap" });
    expect(JSON.parse(asked[2][1]!.body as string).at).toBe(300);
  });
});

describe("recording a sequence", () => {
  it("writes the presses as an input log and says which input is live", async () => {
    const { playedLog, inputHint, recordSays } = await import("../src/scenariofold");
    expect(playedLog([{ pad: 0, polled: true }, { pad: 1 | 128, polled: true }, { pad: 2, polled: false }])).toBe("gp-input-log 0\ncontrollers 1\nP ........\nP R......A\n- ......B.\n");
    expect(inputHint(true)).toBe("Gamepad connected");
    expect(inputHint(false)).toBe("Keyboard: arrows, X = A, Z = B");
    expect(recordSays("ready", 0, 300)).toBe("Record up to 5 seconds from here.");
    expect(recordSays("recording", 60, 300)).toBe("Recording: 4.0 s left.");
    const { RecordSequence } = await import("../src/Scenarios");
    const html = renderToString(<RecordSequence at={300} log={async () => ""} open={async () => Promise.reject(new Error("no"))} save={async () => undefined} />);
    expect(html).toContain("Play it yourself from here");
    expect(html).toContain("Keyboard: arrows");
  });
});
