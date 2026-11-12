// The control room (docs/show.md, "The live cards" and "How many at
// once"): each running job's bars on made-up progress, the cards of jobs
// just done with their change, and the control for how many work at once.
import { describe, expect, it } from "vitest";
import { renderToString } from "react-dom/server";
import { Api } from "../src/api";
import { BackgroundClient, statusSentence, type BgDone, type BgJob, type BgState } from "../src/backfold";
import { doneLine, jobBars, liveCards, workerChoices } from "../src/controlfold";
import { LiveCards, WorkersControl } from "../src/ControlRoom";

const score = (over: Partial<BgJob> = {}): BgJob => ({ kind: "score", clip: "c1", name: "Moss Hop: jump, held 15 frames", slot: 0, frames: 30, total: 121, together: 24, measures: 8, exact: 6, first_wrong: 12, ...over });
const refine: BgJob = { kind: "refine", clip: "c2", name: "Moss Hop: walk right 3 s", slot: 1, round: 2, pass: "impact", impact_k: 4, impact_n: 10, base: 40, best: 10, kept: 0 };
const done = (seq: number, clip: string, before: number | null, after: number, exact = false): BgDone => ({ seq, kind: "score", clip, name: clip, before, after, exact });

describe("the bars of a job in progress", () => {
  it("moves a score's bars toward the goal as frames are compared", () => {
    const b = Object.fromEntries(jobBars(score()).map((x) => [x.id, x]));
    expect(b.together.value).toBeCloseTo(0.8);
    expect(b.exact).toMatchObject({ value: 0.75, says: "6 of 8" });
    expect(b["first-wrong"]).toMatchObject({ says: "frame 12 of 121" });
    expect(b.compared.says).toBe("30 of 121");
    // With nothing wrong yet the first wrong frame moves with the frames compared.
    const later = Object.fromEntries(jobBars(score({ frames: 90, first_wrong: null })).map((x) => [x.id, x]));
    expect(later["first-wrong"].value).toBeCloseTo(90 / 121);
    expect(later["first-wrong"].says).toBe("none yet, 90 of 121 frames");
    // Bars never pass their goal.
    expect(jobBars(score({ frames: 200, total: 121, together: 300 })).every((x) => (x.value ?? 0) <= 1)).toBe(true);
  });

  it("moves a refine's passes and its best against the start", () => {
    const b = Object.fromEntries(jobBars(refine).map((x) => [x.id, x]));
    expect(b.impact).toMatchObject({ value: 0.4, says: "trial 4 of 10" });
    expect(b.fine.value).toBeUndefined();
    expect(b.best).toMatchObject({ value: 0.75, says: "10 wrong, from 40 at the start" });
    expect(jobBars({ ...refine, pass: "fine", fine_k: 7, fine_knobs: 2 }).find((x) => x.id === "fine")?.says).toBe("7 trials on 2 numbers");
  });
});

describe("the live cards", () => {
  it("shows every running job by worker, then the jobs just done to fill the workers", () => {
    const cards = liveCards([refine, score()], [done(1, "a", 500, 600), done(2, "c1", 100, 200), done(3, "b", 700, 1000, true)], 4);
    expect(cards.map((c) => [c.state, c.clip])).toEqual([
      ["running", "c1"],
      ["running", "c2"],
      ["done", "b"],
      ["done", "a"],
    ]);
    expect(liveCards([], [done(1, "a", 1, 2), done(2, "a", 2, 3)], 2).map((c) => c.key)).toEqual(["done-2"]);
  });

  it("says a done card's final score and its change since last time", () => {
    expect(doneLine(done(1, "a", 620, 710))).toEqual({ says: "71% together, was 62% (+9)", change: "up" });
    expect(doneLine(done(1, "a", 900, 1000, true))).toEqual({ says: "matches exactly, was 90% (+10)", change: "up" });
    expect(doneLine(done(1, "a", 800, 700)).change).toBe("down");
    expect(doneLine(done(1, "a", null, 700)).says).toBe("70% together (first score)");
    expect(doneLine({ ...done(1, "a", null, 0), error: "no start" }).says).toMatch(/can't be scored/);
  });

  it("draws a card a job with its picture, name and bars, and a done card flipped to its score", () => {
    const st: BgState = { enabled: true, working: true, scores: {}, questions: [], jobs: [score()], done: [done(4, "c9", 500, 1000, true)], workers: 2, max_workers: 7 };
    const html = renderToString(<LiveCards state={st} picture={(c) => `/p/${c}.png`} open={() => {}} />).replace(/<!-- -->/g, "");
    expect(html).toContain('src="/p/c1.png"');
    expect(html).toContain("Moss Hop: jump, held 15 frames");
    for (const id of ["together", "exact", "first-wrong", "compared"]) expect(html).toContain(`data-testid="live-bar-${id}"`);
    expect(html).toContain('aria-valuenow="80"');
    expect(html).toContain('data-state="done"');
    expect(html).toContain("matches exactly, was 50% (+50)");
    // Off, it says so; a failed check says to check the mapping.
    const off = renderToString(<LiveCards state={{ enabled: false, scores: {}, questions: [] }} picture={() => undefined} open={() => {}} />);
    expect(off).toContain("The background work is off");
    const failed = renderToString(<LiveCards state={{ ...st, check: { passed: false, says: "Check the mapping: x doesn't move by its speed." } }} picture={() => undefined} open={() => {}} />);
    expect(failed).toContain("Check the mapping");
    // The ledger's replay, on screen.
    const ok = renderToString(<LiveCards state={{ ...st, verified: { checked: 120, reproduced: 120, failures: [], says: "verified: 120 of 120 results reproduce" } }} picture={() => undefined} open={() => {}} />);
    expect(ok).toContain("verified: 120 of 120 results reproduce");
    const bad = renderToString(
      <LiveCards state={{ ...st, verified: { checked: 3, reproduced: 2, failures: ["the score of c1: the stored result was changed"], says: "verified: 2 of 3 results reproduce" } }} picture={() => undefined} open={() => {}} />,
    ).replace(/<!-- -->/g, "");
    expect(bad).toContain("verified: 2 of 3 results reproduce: the score of c1: the stored result was changed");
  });

  it("says in the background line every job running at once", () => {
    const st: BgState = { enabled: true, working: true, scores: {}, questions: [], jobs: [score(), refine], job: score() };
    expect(statusSentence(st, [], 0)).toBe("Background: 2 jobs at once: scoring Moss Hop: jump, held 15 frames, fixing Moss Hop: walk right 3 s");
  });
});

describe("how many at once", () => {
  it("offers 1 to the most, with the run's number chosen", () => {
    expect(workerChoices(7)).toEqual([1, 2, 3, 4, 5, 6, 7]);
    expect(workerChoices(undefined)).toEqual([1]);
    const html = renderToString(<WorkersControl state={{ enabled: true, scores: {}, questions: [], workers: 3, max_workers: 5 }} set={() => {}} />).replace(/<!-- -->/g, "");
    // A stepper: workers, minus, 3, plus; 1 to 5.
    expect(html).toContain('<span class="stepper-label">workers</span>');
    expect(html).toContain('<output data-testid="ctl-workers">3</output>');
    expect(html).not.toMatch(/data-testid="ctl-workers-less"[^>]*disabled|disabled=""[^>]*data-testid="ctl-workers-less"/);
    const top = renderToString(<WorkersControl state={{ enabled: true, scores: {}, questions: [], workers: 5, max_workers: 5 }} set={() => {}} />);
    expect(top).toMatch(/disabled=""[^>]*data-testid="ctl-workers-more"/);
  });

  it("sets it on the server for the run", async () => {
    const calls: [string, string | undefined, string | undefined][] = [];
    const api = new Api("", (input, init) => {
      calls.push([String(input), init?.method, init?.body as string | undefined]);
      return Promise.resolve(new Response('{"workers":3,"max":7}', { status: 200, headers: { "Content-Type": "application/json" } }));
    });
    expect(await new BackgroundClient(api).setWorkers("hop", 3)).toEqual({ workers: 3, max: 7 });
    expect(calls).toEqual([["/api/capture/hop/background/workers", "POST", '{"workers":3}']]);
    await new BackgroundClient(api).verify("hop", 10);
    expect(calls[1]).toEqual(["/api/capture/hop/background/verify", "POST", '{"sample":10}']);
  });
});
