// The live panel (docs/capture-live.md): a capture run's events folded into
// stages, numbers, rounds, questions, gaps, pins and loop tables; the
// stream it follows and the polling it falls back to; the requests it
// sends; and that it draws what it's given.
import { describe, expect, it } from "vitest";
import { renderToString } from "react-dom/server";
import { Api, type CaptureEvent, type EventSourceLike } from "../src/api";
import { delta, emptyLive, fold, logLine, openGaps, openQuestions, wontModel } from "../src/livefold";
import { LiveBody, personInput, subscribe, type Transport } from "../src/Live";
import { readUrl, writeUrl } from "../src/App";

let seq = 0;
const ev = (round: number, kind: string, stage?: string, data?: Record<string, unknown>, actor: CaptureEvent["actor"] = "pipeline"): CaptureEvent => ({
  v: 1,
  seq: ++seq,
  at_ms: seq * 10,
  round,
  actor,
  kind,
  stage,
  data,
});

const question = { id: "derive:levels.main.kill_below", text: "kill_below is 200 in the model, the code reads 208", path: "levels.main.kill_below", candidates: [{ from: "the model", value: "200" }, { from: "gp model derive --from evidence", value: "208" }] };

const round1 = [
  ev(0, "run.start", undefined, { run: "demo", model: "templates/mosshop" }),
  ev(1, "round.start"),
  ev(1, "stage.start", "lift"),
  ev(1, "item", "lift", { what: "asset", id: "tiles", status: "lifted" }),
  ev(1, "stage.end", "lift", { status: "ok", ms: 58, summary: "5 assets" }),
  ev(1, "metric", "lift", { key: "assets.count", value: 5, prev: null }),
  ev(1, "stage.end", "derive", { status: "ok", ms: 300, summary: "34 of 95 derived, 1 differ" }),
  ev(1, "metric", "derive", { key: "facts.derived_differ", value: 1, prev: null }),
  ev(1, "question", "questions", question),
  ev(1, "stage.end", "validate", { status: "fail", ms: 1500, summary: "19 pass, 2 fail" }),
  ev(1, "metric", "validate", { key: "fidelity.interp.failed", value: 2, prev: null }),
  ev(1, "round.end", undefined, { ms: 2900, failed: 1, deltas: { "fidelity.interp.failed": { value: 2, prev: null } } }),
];
const round2 = [
  ev(2, "round.start"),
  ev(2, "input", undefined, { kind: "answer", status: "applied", summary: "derive:levels.main.kill_below = candidate 2", input: { kind: "answer", question: question.id, choice: 2 } }, "person"),
  ev(2, "stage.end", "lift", { status: "cached", ms: 0, summary: "5 assets" }),
  ev(2, "stage.start", "validate"),
];
const round2end = [
  ev(2, "stage.end", "validate", { status: "ok", ms: 1490, summary: "21 pass, 0 fail" }),
  ev(2, "metric", "validate", { key: "fidelity.interp.failed", value: 0, prev: 2 }),
  ev(2, "round.end", undefined, { ms: 2500, failed: 0, deltas: { "fidelity.interp.failed": { value: 0, prev: 2 } } }),
];

describe("folding a run's events", () => {
  it("keeps each stage's latest state, the numbers and the rounds", () => {
    const s1 = fold(emptyLive(), round1);
    expect(s1.order).toEqual(["lift", "derive", "validate"]);
    expect(s1.stages.validate.status).toBe("fail");
    expect(s1.metrics["fidelity.interp.failed"].value).toBe(2);
    expect(s1.rounds).toHaveLength(1);
    expect(s1.running).toBe(false);
    expect(openQuestions(s1).map((q) => q.id)).toEqual([question.id]);

    const s2 = fold(s1, round2);
    expect(s2.running).toBe(true);
    expect(s2.stages.lift.status).toBe("cached");
    expect(s2.stages.validate.status).toBe("running");
    expect(openQuestions(s2)).toHaveLength(0);

    const s3 = fold(s2, round2end);
    expect(s3.metrics["fidelity.interp.failed"]).toMatchObject({ value: 0, prev: 2, round: 2 });
    expect(s3.rounds.map((r) => r.failed)).toEqual([1, 0]);
    expect(s3.log.some((e) => e.kind === "metric")).toBe(false);
  });

  it("ignores events it has already folded", () => {
    const s1 = fold(emptyLive(), round1);
    expect(fold(s1, round1)).toEqual(s1);
  });

  it("writes deltas and log lines", () => {
    expect(delta({ value: 0, prev: 2 })).toBe("-2");
    expect(delta({ value: 0.5, prev: 0.25 })).toBe("+0.250");
    expect(delta({ value: 3, prev: null })).toBe("new");
    expect(delta({ value: 3, prev: 3 })).toBe("");
    expect(logLine(round1[4])).toBe("lift ok in 58 ms: 5 assets");
  });
});

describe("the live panel", () => {
  it("draws the stages, the numbers, the rounds and the open question", () => {
    const api = new Api("");
    const html = renderToString(<LiveBody state={fold(emptyLive(), round1)} run="demo" api={api} send={() => {}} />).replace(/<!-- -->/g, "");
    expect(html).toContain("demo: round 1, idle");
    expect(html).toContain("validate</b> fail");
    expect(html).toContain("fidelity.interp.failed");
    expect(html).toContain("208 (gp model derive --from evidence)");
    expect(html).toContain("pin levels.main.kill_below");
    expect(html).not.toContain("<iframe");
  });

  it("draws the preview once a round built one", () => {
    const api = new Api("");
    const s = fold(emptyLive(), [ev(1, "stage.end", "preview", { status: "ok", ms: 900 })]);
    const html = renderToString(<LiveBody state={s} run="demo" api={api} send={() => {}} />);
    expect(html).toContain('src="/api/capture/demo/preview/index.html?files=here"');
  });

  it("asks for events after the last one and posts inputs as JSON", async () => {
    const calls: { url: string; init?: RequestInit }[] = [];
    const api = new Api("", (input, init) => {
      calls.push({ url: String(input), init });
      return Promise.resolve(new Response(JSON.stringify({ events: [], inputs: 1 }), { status: 200 }));
    });
    await api.captureEvents("demo", 42);
    await api.captureInput("demo", { v: 1, actor: "person", kind: "answer", question: question.id, choice: 2 });
    expect(calls[0].url).toBe("/api/capture/demo/events?after=42");
    expect(calls[1].url).toBe("/api/capture/demo/input");
    expect(calls[1].init?.method).toBe("POST");
    expect(JSON.parse(String(calls[1].init?.body))).toMatchObject({ kind: "answer", choice: 2 });
  });

  it("offers the run's recordings to open beside it", () => {
    const api = new Api("");
    const s = fold(emptyLive(), [ev(1, "item", "record", { what: "recording", id: "8ffbc92bf6f8c09f", status: "recorded", text: "300 frames" })]);
    expect(s.recordings).toEqual([{ id: "8ffbc92bf6f8c09f", text: "300 frames" }]);
    const html = renderToString(<LiveBody state={s} run="demo" api={api} send={() => {}} open={() => {}} />).replace(/<!-- -->/g, "");
    expect(html).toContain("8ffbc92b (300 frames)");
  });

  it("keeps the run in the URL", () => {
    expect(readUrl("?a=ab12&run=demo&frame=3").run).toBe("demo");
    expect(writeUrl({ a: "ab12", run: "demo", frame: 3 })).toBe("?a=ab12&run=demo&frame=3");
    expect(writeUrl({ a: "ab12", frame: 3 })).toBe("?a=ab12&frame=3");
  });
});

/** A stand-in EventSource the test drives. */
class FakeSource implements EventSourceLike {
  onmessage: EventSourceLike["onmessage"] = null;
  onerror: EventSourceLike["onerror"] = null;
  closed = false;
  constructor(readonly url: string) {}
  close() {
    this.closed = true;
  }
  push(e: CaptureEvent) {
    this.onmessage?.({ data: JSON.stringify(e), lastEventId: String(e.seq) });
  }
}

const tick = () => new Promise((r) => setTimeout(r, 0));

describe("following a run", () => {
  it("takes the stream, passes each event once and falls back to polling from the last one", async () => {
    const sources: FakeSource[] = [];
    const urls: string[] = [];
    const later = [ev(1, "stage.end", "derive", { status: "ok", ms: 3, summary: "" })];
    const api = new Api(
      "",
      (input) => {
        urls.push(String(input));
        return Promise.resolve(new Response(JSON.stringify({ events: later }), { status: 200 }));
      },
      (url) => {
        const s = new FakeSource(url);
        sources.push(s);
        return s;
      },
    );
    const got: number[] = [];
    const transports: Transport[] = [];
    const stop = subscribe(api, "demo", { events: (evs) => got.push(...evs.map((e) => e.seq)), transport: (t) => transports.push(t), error: () => {} }, 10_000);
    expect(sources[0].url).toBe("/api/capture/demo/stream?after=0");
    const [a, b] = round1;
    sources[0].push(a);
    sources[0].push(b);
    sources[0].push(a);
    expect(got).toEqual([a.seq, b.seq]);
    expect(urls).toEqual([]);
    // The stream fails: the panel asks for what came after the last event.
    sources[0].onerror?.({});
    await tick();
    expect(sources[0].closed).toBe(true);
    expect(transports).toEqual(["stream", "poll"]);
    expect(urls).toEqual([`/api/capture/demo/events?after=${b.seq}`]);
    expect(got).toEqual([a.seq, b.seq, later[0].seq]);
    stop();
  });

  it("polls where there's no EventSource", async () => {
    const urls: string[] = [];
    const api = new Api(
      "",
      (input) => {
        urls.push(String(input));
        return Promise.resolve(new Response(JSON.stringify({ events: [] }), { status: 200 }));
      },
      undefined,
    );
    const transports: Transport[] = [];
    const stop = subscribe(api, "demo", { events: () => {}, transport: (t) => transports.push(t), error: () => {} }, 10_000);
    await tick();
    expect(transports).toEqual(["poll"]);
    expect(urls).toEqual(["/api/capture/demo/events?after=0"]);
    stop();
  });
});

describe("pins", () => {
  const gapEvents = [
    ev(3, "item", "gaps", { what: "gap", id: "ram:0x0400", status: "open", text: "ram" }),
    ev(3, "item", "gaps", { what: "gap", id: "ram:0x0401", status: "open", text: "ram" }),
  ];
  const pin = (path: string, kind = "pin") => ev(4, "input", undefined, { kind, status: "applied", summary: `${path} pinned`, input: { v: 1, actor: "person", kind, path } }, "person");

  it("a pinned question or gap leaves the open lists and is shown as won't model", () => {
    const s = fold(emptyLive(), [...round1, ...gapEvents]);
    expect(openQuestions(s)).toHaveLength(1);
    expect(openGaps(s).map((g) => g.id)).toEqual(["ram:0x0400", "ram:0x0401"]);
    const p = fold(s, [pin("levels.main.kill_below"), pin("ram:0x0400")]);
    expect(p.pins).toEqual(["levels.main.kill_below", "ram:0x0400"]);
    expect(openQuestions(p)).toHaveLength(0);
    expect(openGaps(p).map((g) => g.id)).toEqual(["ram:0x0401"]);
    expect(wontModel(p).map((w) => w.id)).toEqual([question.id, "ram:0x0400"]);
    const html = renderToString(<LiveBody state={p} run="demo" api={new Api("")} send={() => {}} />).replace(/<!-- -->/g, "");
    expect(html).toContain("Won&#x27;t model (2)");
    expect(html).toContain("unpin");
    expect(html).not.toContain("pin levels.main.kill_below");
    const u = fold(p, [pin("ram:0x0400", "unpin")]);
    expect(openGaps(u).map((g) => g.id)).toEqual(["ram:0x0400", "ram:0x0401"]);
  });

  it("sends pins and unpins as inputs of the current format", () => {
    expect(personInput({ kind: "pin", path: "ram:0x0400" })).toEqual({ v: 1, actor: "person", kind: "pin", path: "ram:0x0400" });
    expect(personInput({ kind: "unpin", path: "ram:0x0400" }).v).toBe(1);
  });
});

describe("the watcher and the loops", () => {
  it("shows what the watcher does, cached stages' results and the loops' rows", () => {
    const s = fold(emptyLive(), [
      ev(1, "stage.end", "validate", { status: "cached", ms: 0, summary: "19 pass, 2 fail", result: "fail", from_run: "first" }),
      ev(1, "row", "refine", { table: "refine", index: 1, cells: { failed: 2, applied: 1 } }),
      ev(1, "row", "refine", { table: "refine", index: 2, cells: { failed: 0, applied: 0 } }),
      ev(1, "metric", "preview", { key: "speed.rom_to_preview_s", value: 72.5, prev: null, budget: 60 }),
      ev(1, "run.state", undefined, { state: "waiting", reason: "1 open" }),
    ]);
    expect(s.stages.validate).toMatchObject({ status: "cached", result: "fail", fromRun: "first" });
    expect(s.tables.refine.map((r) => r.cells.failed)).toEqual([2, 0]);
    expect(s.watch).toEqual({ state: "waiting", reason: "1 open" });
    expect(s.metrics["speed.rom_to_preview_s"].budget).toBe(60);
    const html = renderToString(<LiveBody state={s} run="demo" api={new Api("")} send={() => {}} />).replace(/<!-- -->/g, "");
    expect(html).toContain("demo: round 1, waiting");
    expect(html).toContain("validate</b> cached (fail)");
    expect(html).toContain("refine rounds");
    expect(html).toMatch(/class="[^"]*\bover\b/);
    expect(html).toContain("budget 60");
    expect(logLine(s.log[s.log.length - 1])).toBe("run waiting: 1 open");
  });
});
