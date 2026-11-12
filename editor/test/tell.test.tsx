// Tell the agent (docs/show.md, "Telling the agent"): the drawer's input
// and send button, the point clicked on the screen, the speech-bubble
// badge on a clip with an open tell, and the rows with their replies.
import { describe, expect, it } from "vitest";
import { renderToString } from "react-dom/server";
import { Api } from "../src/api";
import { type WorkState, type WorkTell, WorkClient, clipTells, openTells, pinWords, tellBody } from "../src/agentfold";
import { type Ctx, clipBadges, inProgress, needsYouClips, thumbOrder } from "../src/calmfold";
import { BadgeChip, ClipDetail, type ClipDetailProps, Thumbs } from "../src/Calm";
import { ScreenPointer } from "../src/PointTell";
import { CLICK, boxFrom, screenPoint } from "../src/point";

const tell = (id: number, clip: string, extra: Partial<WorkTell> = {}): WorkTell => ({ id, clip, text: "the hop is too low", by: "user", when_ms: id, open: true, replies: [], ...extra });
const work = (tells: WorkTell[]): WorkState => ({ run: "r", working: [], attempts: [], entries: [], tells });

const drawer = (extra: Partial<ClipDetailProps> = {}) =>
  renderToString(
    <ClipDetail
      name="Hop"
      sub="Played by you, 2.0 s"
      status="differs"
      frames={120}
      at={40}
      attempts={[]}
      rounds={[]}
      watch={() => {}}
      keep={() => {}}
      needsYou={[]}
      other={[]}
      asks={[]}
      answer={() => {}}
      mark="kept"
      setMark={() => {}}
      close={() => {}}
      {...extra}
    />,
  );

describe("the drawer's tell box", () => {
  it("has a one-line input and a send button, and says to click the screen", () => {
    const html = drawer({ tell: async () => {}, setPaused: () => {} });
    expect(html).toContain('data-testid="tell-input"');
    expect(html).toContain('data-testid="tell-send"');
    expect(html).toContain("Click the screen to point");
    expect(html).toContain('data-testid="tell-pause"');
    // The send button waits for words.
    expect(html).toMatch(/<button[^>]*disabled=""[^>]*data-testid="tell-send"/);
    // Without the page's handler there is no box.
    expect(drawer()).not.toContain("tell-box");
  });

  it("says where the person pointed, once they did", () => {
    const html = drawer({ tell: async () => {}, pointed: "the spot (120, 88), frame 40", clearPoint: () => {} });
    const flat = html.replace(/<!-- -->/g, "");
    expect(flat).toContain("Pointing at the spot (120, 88), frame 40");
    expect(html).toContain('data-testid="tell-unpoint"');
    expect(html).not.toContain("Click the screen");
  });

  it("shows each note sent as a row with a you-said chip, its reply under it when one comes", () => {
    const open = tell(1, "c");
    const closed = tell(2, "c", { text: "he lands short", frame: 55, open: false, replies: [{ by: "clip agent", text: "Raised the hop; gravity was off.", when_ms: 9 }] });
    const html = drawer({ tell: async () => {}, tells: [open, closed] });
    expect(html.match(/data-testid="tell-row"/g)).toHaveLength(2);
    expect(html.match(/data-testid="you-said">you said</g)).toHaveLength(2);
    expect(html).toContain("waiting for an agent");
    expect(html).toContain("answered");
    expect(html.match(/data-testid="tell-reply"/g)).toHaveLength(1);
    expect(html.replace(/<!-- -->/g, "")).toContain("the clip agent: </span>Raised the hop; gravity was off.");
    // An agent on the clip has it.
    expect(drawer({ tell: async () => {}, tells: [open], agent: { says: "Agent: reading", busy: true } })).toContain("the agent has it");
  });
});

describe("the point clicked on the screen", () => {
  it("is a spot for a click and a region for a drag, in game pixels, with both frames", () => {
    const rect = { left: 100, top: 50, width: 512, height: 480 };
    const p = screenPoint(100 + 240, 50 + 176, rect);
    expect(p).toEqual({ x: 120, y: 88 });
    const click = boxFrom(p, p);
    expect(click.w).toBe(CLICK);
    const body = tellBody("  the hop is too low ", { box: click, click: click.w === CLICK && click.h === CLICK, frame: 40, orig: 1240 });
    expect(body).toEqual({ text: "the hop is too low", frame: 40, orig_frame: 1240, at: [120, 88] });
    const drag = boxFrom({ x: 100, y: 80 }, { x: 123, y: 95 });
    expect(tellBody("lands wrong", { box: drag, click: false, frame: 55 })).toEqual({ text: "lands wrong", frame: 55, region: [100, 80, 24, 16] });
    // No point: only the words.
    expect(tellBody("hello")).toEqual({ text: "hello" });
    expect(pinWords({ box: click, click: true, frame: 40 })).toBe("the spot (120, 88), frame 40");
    expect(pinWords({ box: drag, click: false, frame: 55 })).toBe("a 24x16 region at (100, 80), frame 55");
  });

  it("shows a marker on the screen where they clicked", () => {
    const box = { x: 112, y: 80, w: 16, h: 16 };
    const html = renderToString(
      <ScreenPointer active box={box} onBox={() => {}} testid="screen-point">
        <canvas />
      </ScreenPointer>,
    );
    expect(html).toContain('data-testid="screen-point"');
    expect(html).toContain('class="point-box" x="112" y="80" width="16" height="16"');
    const none = renderToString(
      <ScreenPointer active={false} onBox={() => {}} testid="screen-point">
        <canvas />
      </ScreenPointer>,
    );
    expect(none).not.toContain("point-layer");
  });

  it("is sent to the clip's tell endpoint", async () => {
    const calls: [string, string | undefined, string | undefined][] = [];
    const api = new Api("", (input, init) => {
      calls.push([String(input), init?.method, init?.body as string | undefined]);
      return Promise.resolve(new Response("{}", { status: 200, headers: { "Content-Type": "application/json" } }));
    });
    await new WorkClient(api).tell("hop", "c 1", { text: "x", frame: 3, at: [1, 2] });
    expect(calls[0][0]).toBe("/api/capture/hop/clips/c%201/tell");
    expect(calls[0][1]).toBe("POST");
    expect(JSON.parse(calls[0][2]!)).toEqual({ text: "x", frame: 3, at: [1, 2] });
  });
});

describe("the badge on a clip with an open tell", () => {
  const w = work([tell(1, "clip.a"), tell(2, "clip.b", { open: false, replies: [{ by: "driver", text: "done", when_ms: 3 }] })]);
  const ctx: Ctx = { work: w, now: 0 };

  it("is a speech bubble, not amber, and not a need for the person", () => {
    const kinds = (id: string) => clipBadges(ctx, { id }).map((b) => b.kind);
    expect(kinds("clip.a")).toEqual(["tell"]);
    expect(kinds("clip.b")).toEqual([]);
    expect(needsYouClips(undefined, w).size).toBe(0);
    const html = renderToString(<BadgeChip b={{ kind: "tell", label: "told" }} />);
    expect(html).toContain('data-testid="badge-tell"');
    expect(html).toContain('class="bubble"');
    expect(html).not.toContain("needs-you");
  });

  it("puts the clip in progress, among the clips being worked on", () => {
    expect(inProgress(ctx, { id: "clip.a" })).toBe(true);
    expect(inProgress(ctx, { id: "clip.b" })).toBe(false);
    const order = thumbOrder(ctx, [{ clip: { id: "clip.c" } }, { clip: { id: "clip.a" } }]).map((c) => c.clip.id);
    expect(order).toEqual(["clip.a"]);
    const html = renderToString(<Thumbs thumbs={[{ id: "clip.a", name: "A", picture: "", badges: clipBadges(ctx, { id: "clip.a" }) }]} open={() => {}} />);
    expect(html).toContain('data-testid="badge-tell"');
  });

  it("is kept after the working badge and needs-you", () => {
    const busy: Ctx = { work: { ...w, working: [{ by: "driver", clip: "clip.a", when_ms: 1, says: "x" }] }, now: 0 };
    expect(clipBadges(busy, { id: "clip.a" }).map((b) => b.kind)).toEqual(["agent", "tell"]);
  });

  it("is listed by clip", () => {
    expect(clipTells(w, "clip.a").map((t) => t.id)).toEqual([1]);
    expect(openTells(w).map((t) => t.id)).toEqual([1]);
    expect(openTells(w, "clip.b")).toEqual([]);
  });
});
