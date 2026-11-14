// Pause, point and tell (docs/point-and-tell.md): the pause key, a click
// or drag turned into a box on the picture, the request a Tell sends and
// the panels drawn. The games and words are made up.
import { describe, expect, it } from "vitest";
import { renderToString } from "react-dom/server";
import { Api } from "../src/api";
import { ScreenPointer, TellPanel } from "../src/PointTell";
import { CHOICES, CLICK, boxFrom, choiceWords, isPauseKey, requestInput, screenPoint, where, type PointJson } from "../src/point";

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
