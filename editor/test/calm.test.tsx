// The calm main screen (docs/show.md, "The main screen"): which questions
// need the person, the badges on a thumbnail, the order of the
// thumbnails, a clip's status chip and where it parts, and the header's
// gauge, on made-up states.
import { describe, expect, it } from "vitest";
import { renderToString } from "react-dom/server";
import type { BgQuestion, BgState } from "../src/backfold";
import type { WorkState } from "../src/agentfold";
import { type Ctx, agentAsks, attemptRow, roundRows, clipBadges, clipStatus, exactWords, inProgress, needsYouClips, questionNeedsYou, thumbOrder, whereItParts } from "../src/calmfold";
import { FidelityGauge, Thumbs } from "../src/Calm";
import type { FidelityState } from "../src/Fidelity";

const q = (id: string, clip: string, extra: Partial<BgQuestion> & { kind?: string } = {}): BgQuestion => ({
  id,
  clip,
  clip_name: clip,
  ask: "Watch the clip: how does the rebuilt game look where the two part?",
  choices: ["too fast", "too slow", "looks right"],
  detail: "",
  model: "m",
  ...extra,
});
const base: BgState = { enabled: true, scores: {}, questions: [] };
const work = (extra: Partial<WorkState> = {}): WorkState => ({ run: "r", working: [], attempts: [], entries: [], ...extra });
const card = (id: string, permille?: number) => ({ clip: { id, score: permille === undefined ? undefined : { exact: permille === 1000, permille } } });

describe("what needs you", () => {
  it("counts a conflict, a question on a clip you recorded, and an agent's ask; not the loop's generic question on a clip the tool made", () => {
    expect(questionNeedsYou(q("1", "gen.a"))).toBe(false);
    expect(questionNeedsYou(q("2", "gen.a", { kind: "conflict", choices: ["check the mapping", "the code is wrong here"] }))).toBe(true);
    // Older questions have no kind: a conflict is read by its choices.
    expect(questionNeedsYou(q("3", "gen.a", { choices: ["check the mapping", "the code is wrong here"] }))).toBe(true);
    expect(questionNeedsYou(q("4", "gen.a", { kind: "shape" }))).toBe(false);
    expect(questionNeedsYou(q("5", "clip.1793.2"))).toBe(true);
    expect(questionNeedsYou(q("6", "clip.1793.2", { answer: "too fast" }))).toBe(false);
    const w = work({ entries: [{ kind: "ask", by: "clip agent", clip: "gen.b", text: "Is Mario meant to bounce here?", when_ms: 5 }] });
    expect(agentAsks(w).map((e) => e.clip)).toEqual(["gen.b"]);
    // Answered by the person, or the work on it done, it no longer asks.
    expect(agentAsks(work({ entries: [...w.entries, { kind: "note", by: "you", clip: "gen.b", text: "yes", when_ms: 6 }] }))).toEqual([]);
    expect(agentAsks(work({ entries: [...w.entries, { kind: "done", by: "clip agent", clip: "gen.b", when_ms: 7 }] }))).toEqual([]);
    const bg = { ...base, questions: [q("1", "gen.a"), q("2", "gen.c", { kind: "conflict" }), q("5", "clip.9.1")] };
    expect([...needsYouClips(bg, w)].sort()).toEqual(["clip.9.1", "gen.b", "gen.c"]);
  });
});

describe("the badges on a thumbnail", () => {
  it("shows at most two, needs you first, then working, then better; stuck only when nothing else is on it", () => {
    const bg: BgState = {
      ...base,
      questions: [q("1", "clip.1.1")],
      jobs: [{ kind: "refine", clip: "clip.1.1", name: "c" }, { kind: "score", clip: "gen.s", name: "s" }],
      clip_agents: [{ clip: "gen.lead", clips: ["gen.lead", "gen.group"], state: "working" }],
      done: [{ seq: 1, kind: "refine", clip: "clip.1.1", name: "c", before: 600, after: 800, exact: false }, { seq: 2, kind: "score", clip: "gen.up", name: "u", before: 500, after: 900, exact: false }],
      stuck: { "gen.stuck": "m", "gen.group": "m" },
    };
    const ctx: Ctx = { bg, now: 0 };
    const kinds = (id: string, s?: { exact: boolean; permille: number }) => clipBadges(ctx, { id, score: s }).map((b) => b.kind);
    // Needs you, fixing and better: only the first two.
    expect(kinds("clip.1.1")).toEqual(["needs-you", "fixing"]);
    expect(kinds("gen.s")).toEqual(["scoring"]);
    // A clip in an agent's group shows the agent, not stuck.
    expect(kinds("gen.group")).toEqual(["agent"]);
    expect(kinds("gen.lead")).toEqual(["agent"]);
    expect(kinds("gen.up")).toEqual(["better"]);
    expect(kinds("gen.stuck")).toEqual(["stuck"]);
    expect(kinds("gen.stuck", { exact: true, permille: 1000 })).toEqual([]);
    expect(kinds("gen.quiet")).toEqual([]);
    // The generic question on a clip the tool made is no badge.
    expect(clipBadges({ bg: { ...base, questions: [q("1", "gen.q")] }, now: 0 }, { id: "gen.q" })).toEqual([]);
    // Got better on this page: its trend rose and it changed lately.
    expect(clipBadges({ now: 1000, changed: { "gen.t": 900 } }, { id: "gen.t" }, [400, 700]).map((b) => b.kind)).toEqual(["better"]);
    expect(clipBadges({ now: 1000 + 11 * 60 * 1000, changed: { "gen.t": 900 } }, { id: "gen.t" }, [400, 700])).toEqual([]);
  });
});

describe("the thumbnails' order", () => {
  it("keeps only clips in progress and sorts needs you first, then working, then better, then the rest changed lately", () => {
    const bg: BgState = {
      ...base,
      questions: [q("1", "clip.9.1"), q("2", "gen.generic")],
      jobs: [{ kind: "refine", clip: "gen.job", name: "j" }],
      clip_agents: [{ clip: "gen.agent", state: "working" }],
      done: [{ seq: 1, kind: "score", clip: "gen.better", name: "b", before: 100, after: 200, exact: false }],
    };
    const ctx: Ctx = { bg, now: 5000, changed: { "gen.moved": 4000, "gen.old": 1 - 20 * 60 * 1000 } };
    const cards = ["gen.quiet", "gen.moved", "gen.better", "gen.agent", "gen.job", "clip.9.1", "gen.generic", "gen.old"].map((id) => card(id, 500));
    expect(thumbOrder(ctx, cards).map((c) => c.clip.id)).toEqual(["clip.9.1", "gen.agent", "gen.job", "gen.better", "gen.moved"]);
    expect(inProgress(ctx, { id: "gen.generic" })).toBe(false);
  });

  it("draws needs you loud, with its count, and each thumbnail's bar and badges", () => {
    const html = renderToString(
      <Thumbs
        thumbs={[
          { id: "a", name: "World 1-1: walk", picture: "/a.png", permille: 620, badges: [{ kind: "needs-you", label: "needs you" }] },
          { id: "b", name: "World 1-2", picture: "", permille: 1000, badges: [{ kind: "agent", label: "agent" }] },
        ]}
        open={() => {}}
        more={4}
      />,
    ).replace(/<!-- -->/g, "");
    expect(html).toContain('data-testid="needs-you-count">1 need you<');
    expect(html).toMatch(/class="thumb needs-you"[^>]*data-clip="a"/);
    expect(html).toContain('data-testid="badge-agent"><span class="spin"');
    expect(html).toContain('aria-valuenow="62"');
    expect(html).toContain("+4 more");
  });
});

describe("a clip's detail", () => {
  it("chips its status: needs you, being fixed, matches, close, differs, waiting", () => {
    const bg: BgState = { ...base, questions: [q("1", "clip.1.1")], jobs: [{ kind: "refine", clip: "gen.f", name: "f" }] };
    const ctx: Ctx = { bg, now: 0 };
    expect(clipStatus(ctx, { id: "clip.1.1", score: { exact: false, permille: 10 } })).toBe("needs you");
    expect(clipStatus(ctx, { id: "gen.f", score: { exact: false, permille: 10 } })).toBe("being fixed");
    expect(clipStatus(ctx, { id: "gen.m", score: { exact: true, permille: 1000 } })).toBe("matches");
    expect(clipStatus(ctx, { id: "gen.c", score: { exact: false, permille: 900 } })).toBe("close");
    expect(clipStatus(ctx, { id: "gen.d", score: { exact: false, permille: 899 } })).toBe("differs");
    expect(clipStatus(ctx, { id: "gen.w" })).toBe("waiting");
  });

  it("names where it parts: the earliest track, or the drift", () => {
    const s = { exact: false, permille: 600, tracks: [{ track: "camera", frame: 40, path: "cameras.main.x", says: "" }, { track: "mario", frame: 9, path: "entities.mario[0].vx", says: "" }] };
    expect(whereItParts(undefined, s)).toMatchObject({ frame: 9 });
    expect(whereItParts(undefined, { exact: false, permille: 600, drift: 12, drift_path: null })).toEqual({ frame: 12, what: "the two games" });
    expect(whereItParts(undefined, { exact: true, permille: 1000 })).toBeUndefined();
  });

  it("makes an attempt one row: title, before and after, better and worse counts", () => {
    const side = (permille: number) => ({ permille, score: 1, exact: false, says: "" });
    const a = { n: 3, by: "clip agent", clip: "c", title: "Mario walks under the flagpole.", before: side(960), after: { ...side(1000), exact: true }, better: ["x"], worse: ["y", "z"], others: 3, when_ms: 0 };
    expect(attemptRow(a)).toEqual({ title: "Mario walks under the flagpole", before: 960, after: 1000, better: 1, worse: 2, busy: false });
    expect(attemptRow({ ...a, after: null }).busy).toBe(true);
  });
});

describe("the header's fidelity gauge", () => {
  it("shows the score in a ring, three small bars, and the clips exact as 170 / 256", () => {
    const f = {
      version: "fidelity 1",
      score: 74,
      behaviour: { score: 80, ppm: 0, clips: 256, exact: 170 },
      coverage: { score: 67, ppm: 0 },
      picture: null,
      confidence: { level: "medium", says: "Medium evidence" },
      holding: [],
      says: "",
      trend: "",
      history: [],
    } as FidelityState;
    expect(exactWords(f)).toBe("170 / 256");
    const html = renderToString(<FidelityGauge f={f} />);
    expect(html).toContain('data-testid="fidelity-exact">170 / 256<');
    expect(html).toMatch(/data-testid="fidelity-score">74</);
    expect(html).toContain('data-testid="fidelity-behaviour"');
    expect(html).toContain('<span class="fid-num">-</span>');
  });
});

describe("the rounds in Being tried", () => {
  it("shows a run of stuck rounds once, newest first, at most three", () => {
    const r = (round: number, extra = {}) => ({ round, when_ms: round, says: "", ...extra });
    const rows = [r(9, { stuck: true }), r(8, { stuck: true }), r(7, { changes: [{}] }), r(6, { stuck: true }), r(5, { changes: [{}] })];
    expect(roundRows(rows).map((x) => x.round)).toEqual([9, 7, 6]);
    expect(roundRows([{ when_ms: 0, says: "You said too high." }])).toEqual([]);
  });

  it("names a measure the way a person says it", () => {
    expect(whereItParts(undefined, { exact: false, permille: 600, drift: 50, drift_path: "entities.mario[0].vx" })?.what).toBe("the mario's speed across");
  });
});

describe("the background line's count", () => {
  it("counts only the questions that need you", async () => {
    const { statusSentence } = await import("../src/backfold");
    const st: BgState = { ...base, working: false, enabled: false, questions: [q("1", "gen.a"), q("2", "gen.b"), q("3", "clip.1.1")] };
    expect(statusSentence(st, [], 0)).toBe("Background: off; turn it on to score your clips and fix numbers while you play; 1 question for you");
  });
});
