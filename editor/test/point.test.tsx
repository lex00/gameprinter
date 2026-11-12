// Pause, point and tell (docs/point-and-tell.md): the pause key, a click
// or drag turned into a box on the picture, the request a Tell sends, the
// requests folded from the run's events with their replies, and the
// panels drawn. The games and words are made up.
import { describe, expect, it } from "vitest";
import { renderToString } from "react-dom/server";
import { Api, type CaptureEvent } from "../src/api";
import { emptyLive, fold, logLine } from "../src/livefold";
import { RequestList, ScreenPointer, TellPanel } from "../src/PointTell";
import { CHOICES, CLICK, boxFrom, choiceWords, foldRequests, isPauseKey, requestInput, screenPoint, statusWords, where, type PointJson } from "../src/point";

let seq = 0;
const ev = (kind: string, data: Record<string, unknown>, actor: CaptureEvent["actor"] = "person"): CaptureEvent => ({ v: 1, seq: ++seq, at_ms: 0, round: 1, actor, kind, data });

const pointed: PointJson = {
  run: "pond",
  kind: "play",
  recording: "session",
  recording_id: "ab12",
  frame: 300,
  model_frame: 280,
  rect: { x: 120, y: 144, w: 16, h: 16 },
  label: "a lily pad",
  things: [
    { what: "cell", label: "a lily pad", name: "lily pad", role: "interactive" },
    { what: "entity", label: "the heron", kind: "heron", original: 1, model: 0 },
  ],
  original: { world: { x: 1264, y: 144, w: 16, h: 16 }, camera: { x: 1144 } },
  model: { world: { x: 120, y: 144, w: 16, h: 16 }, camera: [0, 0], entities: [] },
};

describe("pause", () => {
  it("Space and P pause, except while typing", () => {
    expect(isPauseKey({ code: "Space" })).toBe(true);
    expect(isPauseKey({ code: "KeyP" })).toBe(true);
    expect(isPauseKey({ code: "KeyX" })).toBe(false);
    expect(isPauseKey({ code: "Space", target: { tagName: "TEXTAREA" } })).toBe(false);
    expect(isPauseKey({ code: "KeyP", target: { tagName: "INPUT" } })).toBe(false);
    expect(isPauseKey({ code: "KeyP", target: { tagName: "CANVAS" } })).toBe(true);
  });
});

describe("point", () => {
  it("maps the pointer to the picture however big it's drawn", () => {
    const r = { left: 10, top: 20, width: 512, height: 480 };
    expect(screenPoint(10, 20, r)).toEqual({ x: 0, y: 0 });
    expect(screenPoint(10 + 256, 20 + 240, r)).toEqual({ x: 128, y: 120 });
    expect(screenPoint(9999, 9999, r)).toEqual({ x: 255, y: 239 });
    expect(screenPoint(-5, -5, r)).toEqual({ x: 0, y: 0 });
  });

  it("turns a click into a cell-sized box and a drag into its rectangle", () => {
    expect(boxFrom({ x: 100, y: 100 }, { x: 101, y: 100 })).toEqual({ x: 100 - CLICK / 2, y: 100 - CLICK / 2, w: CLICK, h: CLICK });
    // A click at the edge stays on the picture.
    expect(boxFrom({ x: 2, y: 238 }, { x: 2, y: 238 })).toEqual({ x: 0, y: 240 - CLICK, w: CLICK, h: CLICK });
    // Dragged up and left: the same box as down and right.
    expect(boxFrom({ x: 60, y: 90 }, { x: 20, y: 40 })).toEqual({ x: 20, y: 40, w: 41, h: 51 });
    expect(boxFrom({ x: 250, y: 230 }, { x: 255, y: 239 })).toEqual({ x: 250, y: 230, w: 6, h: 10 });
  });

  it("says which games have a thing", () => {
    expect(where(pointed.things[1])).toBe("only in the original");
    expect(where({ what: "entity", label: "the frog", original: 1, model: 1 })).toBe("in both games");
    expect(where(pointed.things[0])).toBe("");
  });
});

describe("tell", () => {
  it("sends the frame, the region in both games, the things and the words", () => {
    const i = requestInput(pointed, "react", "  it should sink when I land  ");
    expect(i.kind).toBe("request");
    expect(i.actor).toBe("person");
    expect(i.text).toBe("it should sink when I land");
    const d = i.data as Record<string, any>;
    expect(d.frame).toBe(300);
    expect(d.recording).toBe("session");
    expect(d.choice).toBe("react");
    expect(d.label).toBe("a lily pad");
    expect(d.point.original.world).toEqual({ x: 1264, y: 144, w: 16, h: 16 });
    expect(d.point.model.world).toEqual({ x: 120, y: 144, w: 16, h: 16 });
    expect(d.point.things).toHaveLength(2);
    expect(requestInput(pointed, undefined, "x").data?.choice).toBeNull();
  });

  it("has the five quick choices, worded for a person", () => {
    expect(CHOICES.map((c) => c.key)).toEqual(["missing", "position", "look", "sound", "react"]);
    expect(choiceWords("react")).toBe("should react when I hit it");
    expect(choiceWords("nope")).toBeUndefined();
  });

  it("asks for a box before anything else, then names what's there", () => {
    const api = new Api("", async () => new Response("{}"));
    const before = renderToString(<TellPanel api={api} run="pond" recording="session" frame={300} />);
    expect(before).toContain('data-testid="tell-hint"');
    expect(before).toContain("drag a box");
    const html = renderToString(<TellPanel api={api} run="pond" recording="session" frame={300} box={{ x: 1, y: 2, w: 16, h: 16 }} />);
    expect(html).toContain("what should happen here?");
    for (const c of CHOICES) expect(html).toContain(c.words);
    // Nothing to send until the box is resolved and something is said.
    expect(html).toMatch(/<button[^>]*disabled=""[^>]*data-testid="tell-send"/);
  });

  it("draws the pointing layer only while paused, with the box", () => {
    const off = renderToString(
      <ScreenPointer active={false} onBox={() => {}} testid="layer">
        <canvas />
      </ScreenPointer>,
    );
    expect(off).not.toContain("point-layer");
    const on = renderToString(
      <ScreenPointer active box={{ x: 3, y: 4, w: 16, h: 16 }} onBox={() => {}} testid="layer">
        <canvas />
      </ScreenPointer>,
    );
    expect(on).toContain('data-testid="layer"');
    expect(on).toContain('class="point-box" x="3" y="4" width="16" height="16"');
  });
});

describe("requests", () => {
  const events = [
    ev("request", { id: 1, frame: 300, recording: "session", choice: "react", text: "it should sink", label: "a lily pad", context: { rect: { x: 120, y: 144, w: 16, h: 16 } } }),
    ev("request", { id: 2, frame: 40, recording: "session", choice: "look", text: "", label: "the heron", context: {} }),
    ev("request.reply", { request: 1, reply: 1, status: "needs_mechanic", text: "In the original it sinks.", lines: ["You pointed at a lily pad.", "In the original it sinks.", "What would cover it: an interactive tile kind."], data: { would_build: { piece: "interactive tile kind" } } }, "agent"),
    ev("request.reply", { request: 2, reply: 1, status: "proposed", text: "A new look", lines: ["You pointed at the heron.", "Here's a meaner heron."], data: { proposal: { item: "pond:3", kind: "asset_edit" } } }, "agent"),
  ];

  it("folds requests and replies by number, once each", () => {
    const r = foldRequests({}, [...events, events[2]]);
    expect(Object.keys(r)).toEqual(["1", "2"]);
    expect(r[1].status).toBe("needs_mechanic");
    expect(r[1].replies).toHaveLength(1);
    expect(r[1].replies[0].wouldBuild).toEqual({ piece: "interactive tile kind" });
    expect(r[1].rect).toEqual({ x: 120, y: 144, w: 16, h: 16 });
    expect(r[2].replies[0].proposal?.item).toBe("pond:3");
    // The person keeps the proposed change.
    const kept = foldRequests(r, [ev("input", { kind: "accept", status: "applied", summary: "", input: { kind: "accept", item: "pond:3" } })]);
    expect(kept[2].status).toBe("accepted");
    expect(kept[1].status).toBe("needs_mechanic");
  });

  it("say an agent is working on a request until its reply or a failure comes", () => {
    const api = new Api("", async () => new Response("{}"));
    const working = ev("request.reply", { request: 1, reply: 2, status: "needs_mechanic", text: "An agent is working on this.", data: { agent: "working" } }, "agent");
    const mid = foldRequests(foldRequests({}, events), [working]);
    const show = (r: ReturnType<typeof foldRequests>) => renderToString(<RequestList api={api} run="pond" requests={r} explaining={[]} watch={() => {}} send={() => {}} />);
    expect(mid[1].replies[1].agent).toBe("working");
    expect(show(mid)).toContain("an agent is working on this");
    const failed = foldRequests(mid, [ev("request.reply", { request: 1, reply: 3, status: "needs_mechanic", text: "The agent stopped.", data: { agent: "failed" } }, "agent")]);
    expect(show(failed)).not.toContain("an agent is working on this");
    expect(statusWords("explained", false, true)).toBe("an agent is working on this");
  });

  it("are part of the live state and the log", () => {
    const s = fold(emptyLive(), events);
    expect(s.requests[2].label).toBe("the heron");
    expect(logLine(events[0])).toBe("request 1 at frame 300: a lily pad: it should sink");
    expect(logLine(events[2])).toContain("needs_mechanic");
    // Folding the same events again changes nothing.
    expect(fold(s, events).requests[1].replies).toHaveLength(1);
  });

  it("show each request with its answer, the status in words and a proposal's decision", () => {
    const api = new Api("", async () => new Response("{}"));
    const html = renderToString(<RequestList api={api} run="pond" requests={foldRequests({}, events)} explaining={[]} watch={() => {}} send={() => {}} />);
    expect(html).toContain("What you asked for (2)");
    expect(html).toContain("a lily pad");
    expect(html).toContain("should react when I hit it");
    expect(html).toContain("needs a new kind of part in the model");
    expect(html).toContain("In the original it sinks.");
    // The first line repeats what the person said; it isn't shown again.
    expect(html).not.toContain("You pointed at a lily pad.");
    expect(html).toContain('data-testid="proposal-accept"');
    expect(html).toContain("/api/capture/pond/edits/pond/3/context");
    expect(html).toContain("/api/capture/pond/edits/pond/3/edit");
    expect(statusWords("open", true)).toContain("looking");
    expect(renderToString(<RequestList api={api} run="pond" requests={{}} explaining={[]} watch={() => {}} send={() => {}} />)).toBe("");
  });

  it("show a level tile's proposed look with its pictures and the keep and undo buttons", () => {
    const tile = [
      ev("request", { id: 1, frame: 169, recording: "finish-level", choice: null, text: "make it orange", label: "a stone block", rect: { x: 49, y: 159, w: 17, h: 17 } }),
      ev("request.reply", { request: 1, reply: 1, status: "explained", text: "", lines: ["You pointed at a stone block.", 'The model draws a stone block as the level tile "stone block".'], data: { piece: { kind: "tile look", tile: "stone block", tool: "assets_edit" } } }, "agent"),
      ev("request.reply", { request: 1, reply: 2, status: "proposed", text: "An orange stone block", lines: ["An orange stone block"], data: { proposal: { item: "hop:1", kind: "asset_edit" } } }, "agent"),
    ];
    const api = new Api("", async () => new Response("{}"));
    const html = renderToString(<RequestList api={api} run="demo" requests={foldRequests({}, tile)} explaining={[]} watch={() => {}} send={() => {}} />);
    expect(html).toContain('level tile &quot;stone block&quot;');
    expect(html).toContain("/api/capture/demo/edits/hop/1/edit");
    expect(html).toContain("/api/capture/demo/edits/hop/1/context");
    expect(html).toContain('data-testid="proposal-accept"');
    expect(html).toContain("no, undo it");
  });
});
