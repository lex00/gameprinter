// Claude's work on a clip (docs/show.md, "Claude working on a clip"): the
// Agent section on the clip card, the progress panel's line, and what the
// client sends, on made-up work records.
import { describe, expect, it } from "vitest";
import { renderToString } from "react-dom/server";
import { Api } from "../src/api";
import { AgentProgress, AgentSection } from "../src/Agent";
import { APPROVED, type WorkAttempt, type WorkState, WorkClient, agentLine, attemptLine, attemptNote, progressLines, workingOn } from "../src/agentfold";
import { ClipDrawerFor } from "../src/Unified";

const side = (permille: number, score: number) => ({ permille, score, exact: score === 0, first_wrong: 9, says: "" });

const tried: WorkAttempt = {
  n: 2,
  by: "driver",
  clip: "c1",
  title: "facing only turns on the ground",
  says: "The game reads the facing only while Mario stands.",
  before: side(620, 40),
  after: side(810, 12),
  better: ["a", "b", "c"],
  worse: [],
  others: 5,
  preview: "agent/2/preview",
  when_ms: 2,
  line: "Attempt 2: facing only turns on the ground. Clip: 62% -> 81%; others: 3 better, none worse.",
  keepable: true,
};

const state: WorkState = {
  run: "hop",
  working: [{ by: "driver", clip: "c1", on: "the jump (vy parts at frame 9)", last: "found the cause in JumpEngine", when_ms: 1, says: "Claude is working on: the jump (vy parts at frame 9). Last: found the cause in JumpEngine." }],
  attempts: [{ ...tried, n: 1, title: "gravity by the hold", after: undefined, before: undefined, line: undefined, keepable: false, preview: null, model_says: "" }, tried],
  entries: [],
};

describe("Claude's work on a clip", () => {
  it("says each attempt in a line, with its score change or what it does now", () => {
    expect(attemptLine(tried)).toBe("Attempt 2: facing only turns on the ground. Clip: 62% -> 81%; others: 3 better, none worse.");
    expect(attemptLine(state.attempts[0])).toBe("Attempt 1: gravity by the hold. Building it and scoring the clips with the change now.");
    expect(attemptLine({ ...state.attempts[0], model_says: "It failed: the worktree's gp didn't build" })).toBe("Attempt 1: gravity by the hold. It failed: the worktree's gp didn't build");
    expect(attemptNote(tried)).toBeUndefined();
    expect(attemptNote({ ...tried, shape: true })).toBe("This changes how a mechanic works, so it needs your OK: keep gives it.");
    expect(attemptNote({ ...tried, keepable: false, worse: ["d"] })).toBe("It makes other clips worse, so it can't be kept as it is.");
    expect(attemptNote({ ...tried, approved: true })).toBe(APPROVED);
    expect(APPROVED).toBe("Approved; waiting to be merged into the game (the driver merges and restarts).");
  });

  it("says who is working on what, on the clip and in the progress panel", () => {
    expect(workingOn(state, "c1")).toHaveLength(1);
    expect(workingOn(state, "c2")).toHaveLength(0);
    expect(progressLines(state)).toEqual([{ says: "Claude is working on: the jump (vy parts at frame 9). Last: found the cause in JumpEngine.", by: "the driver" }]);
    const html = renderToString(<AgentProgress state={state} />);
    expect(html).toContain("Claude is working on: the jump (vy parts at frame 9). Last: found the cause in JumpEngine.");
    expect(html).toContain("the driver");
    expect(renderToString(<AgentProgress state={{ ...state, working: [] }} />)).toBe("");
  });

  it("shows the Agent section on the clip card with watch this try and keep", () => {
    const html = renderToString(<AgentSection state={state} clip="c1" watch={() => {}} keep={() => {}} stop={() => {}} />);
    expect(html).toContain('data-testid="clip-agent"');
    expect(html).toContain("Claude");
    expect(html).toContain("the driver");
    expect(html).toContain("Attempt 2: facing only turns on the ground. Clip: 62% -&gt; 81%; others: 3 better, none worse.");
    expect(html.match(/data-testid="agent-attempt"/g)).toHaveLength(2);
    // Only the attempt with a preview can be watched, and only the better one kept.
    expect(html.match(/data-testid="agent-watch"/g)).toHaveLength(1);
    expect(html.match(/data-testid="agent-keep"/g)).toHaveLength(1);
    expect(html).toContain("watch this try");
    // Watching it: the button goes back.
    expect(renderToString(<AgentSection state={state} clip="c1" watching={2} watch={() => {}} keep={() => {}} stop={() => {}} />)).toContain("back to the game as it is");
    // Kept: no keep button, the approved line.
    const kept = { ...state, attempts: [{ ...tried, approved: true }] };
    const k = renderToString(<AgentSection state={kept} clip="c1" watch={() => {}} keep={() => {}} stop={() => {}} />);
    expect(k).not.toContain('data-testid="agent-keep"');
    expect(k).toContain("Approved; waiting to be merged into the game");
    // Nothing on another clip.
    expect(renderToString(<AgentSection state={state} clip="c2" watch={() => {}} keep={() => {}} stop={() => {}} />)).toBe("");
  });

  it("shows the clip agent's status with a busy dot, stop while it works, and start", () => {
    const working = { ...state, working: [], clip_agent: { clip: "c1", state: "working" as const, says: "Agent: trying 'facing only turns on the ground' (attempt 2)" } };
    expect(agentLine(working.clip_agent, "c1")).toEqual({ says: "Agent: trying 'facing only turns on the ground' (attempt 2)", busy: true });
    expect(agentLine(working.clip_agent, "c2")).toBeUndefined();
    const html = renderToString(<AgentSection state={working} clip="c1" watch={() => {}} keep={() => {}} start={() => {}} stop={() => {}} />);
    expect(html).toContain("Agent: trying &#x27;facing only turns on the ground&#x27; (attempt 2)");
    expect(html).toContain("the clip agent");
    expect(html).toContain("busy-dot");
    expect(html).toContain('data-testid="agent-stop"');
    expect(html).not.toContain('data-testid="agent-start"');
    const idle = renderToString(<AgentSection state={{ ...state, working: [], attempts: [] }} clip="c1" watch={() => {}} keep={() => {}} start={() => {}} stop={() => {}} />);
    expect(idle).toContain("let the agent fix it");
  });

  it("lists the attempts in the clip drawer, one row each with its bars, counts, watch and keep", () => {
    const w = { clip: { id: "c1", frames: 120, mark: "kept" as const, source: "played" as const, session: "s1" }, item: { kind: "save" as const, key: "s1/1", session: "s1", save: { id: 1, frame: 60 } } } as unknown as Parameters<typeof ClipDrawerFor>[0]["w"];
    const html = renderToString(
      <ClipDrawerFor w={w} ctx={{ work: state, now: 0 }} score={{ exact: false, permille: 620, drift: 9 }} at={0} mark={() => {}} fix={() => {}} working={false} answer={() => {}} close={() => {}} />,
    ).replace(/<!-- -->/g, "");
    expect(html.match(/data-testid="agent-attempt"/g)).toHaveLength(2);
    // Newest first: attempt 2, with 62% -> 81% as two bars and 3 better, none worse.
    expect(html.indexOf("facing only turns on the ground")).toBeLessThan(html.indexOf("gravity by the hold"));
    expect(html).toContain('aria-valuenow="62"');
    expect(html).toContain('aria-valuenow="81"');
    expect(html).toContain('<span class="good" title="other clips better">+3</span>');
    expect(html).toContain('data-testid="agent-watch"');
    expect(html).toContain('data-testid="agent-keep"');
    // The driver on the clip is one row in "Being tried".
    expect(html).toContain('data-testid="agent-status"');
    expect(html).toContain('data-testid="clip-status">being fixed<');
  });

  it("sends what the server expects", async () => {
    const calls: [string, string | undefined, string | undefined][] = [];
    const api = new Api("", (input, init) => {
      calls.push([String(input), init?.method, init?.body as string | undefined]);
      return Promise.resolve(new Response("{}", { status: 200, headers: { "Content-Type": "application/json" } }));
    });
    const c = new WorkClient(api);
    await c.state("hop");
    await c.approve("hop", 2);
    await c.start("hop", "c 1");
    await c.stop("hop");
    expect(calls.map((x) => x[0])).toEqual(["/api/capture/hop/agent", "/api/capture/hop/agent/approve", "/api/capture/hop/clips/c%201/agent", "/api/capture/hop/agent/stop"]);
    expect(calls[1][2]).toBe('{"attempt":2}');
    expect(c.previewUrl("hop", 2)).toBe("/api/capture/hop/agent/attempts/2/preview/index.html?control=1");
    expect(c.curvePath("hop", 2)).toBe("/api/capture/hop/agent/attempts/2/curve");
  });
});

describe("fix this clip on a stuck clip", () => {
  it("starts the clip agent when the loop is stuck on a question about the clip", async () => {
    const { stuckOnQuestion } = await import("../src/Unified");
    const q = { id: "q1", clip: "c1", clip_name: "Clip 1", ask: "", choices: [], detail: "", model: "m" };
    const base = { enabled: true, scores: {}, questions: [] };
    expect(stuckOnQuestion({ ...base, questions: [q] }, "c1")).toBe(true);
    expect(stuckOnQuestion({ ...base, questions: [{ ...q, answer: "too high" }] }, "c1")).toBe(false);
    expect(stuckOnQuestion({ ...base, stuck: { c1: "m" } } as never, "c1")).toBe(true);
    expect(stuckOnQuestion(base, "c1")).toBe(false);
  });
});
