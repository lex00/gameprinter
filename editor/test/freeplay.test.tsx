// Free play (docs/show.md, "Free play"): the words it shows, the frames
// file lines the rebuilt game gets, the requests it makes, and that the
// panel renders its first sentence before the player loads.
import { describe, expect, it } from "vitest";
import { renderToString } from "react-dom/server";
import { Api } from "../src/api";
import { FreePlay } from "../src/FreePlay";
import { CLIP_SECONDS, FreePlayClient, checkWords, clipCard, clipLength, clock, freeSays, latest, modelLine, secs, takeSays } from "../src/freefold";
import { framesScript } from "../src/playfold";

describe("the words", () => {
  it("tells the next thing to do", () => {
    expect(freeSays({ saves: 0, paused: false })).toMatch(/Press F5/);
    expect(freeSays({ saves: 2, follow: "following", paused: false })).toMatch(/F8 goes back/);
    expect(freeSays({ saves: 2, follow: "none", paused: false })).toMatch(/can't start from this save point yet/);
    expect(freeSays({ saves: 1, paused: true })).toMatch(/Press P/);
    expect(freeSays({ saves: 1, follow: "waiting", paused: false, model: false })).toMatch(/no rebuilt game/);
  });
  it("reads times as a person does", () => {
    expect(clock(0)).toBe("0:00");
    expect(clock(2470)).toBe("0:41");
    expect(secs(373)).toBe("6.2 s");
  });
  it("says what a save point's check found", () => {
    expect(checkWords({ native: null })).toBe("checking");
    expect(checkWords({ native: { hash: "a", digest: "b", matches: true } })).toBe("rebuilt the same");
    expect(checkWords({ native: { hash: "a", digest: "b", matches: false } })).toBe("REBUILT DIFFERENTLY");
    expect(checkWords({ error: "no ROM" })).toMatch(/couldn't/);
  });
  it("walks a take through and says what was saved", () => {
    expect(takeSays("count", 3, 0, 480)).toMatch(/Get ready: 3/);
    expect(takeSays("recording", 3, 120, 480)).toBe("Recording the clip: 2.0 s of 8.0 s. Press stop to end it early.");
    expect(clipCard(373, 2, 2470)).toMatch(/^Clip saved: 6.2 s from save point 2 at 0:41\./);
    expect(clipCard(373, 2, 2470, "World 1-1")).toMatch(/^Clip saved: 6.2 s from World 1-1 at 0:41\./);
  });
  it("keeps the clip length a setting between 1 and 30 seconds", () => {
    expect(CLIP_SECONDS).toBe(8);
    expect(clipLength(5)).toBe(5);
    expect(clipLength(0)).toBe(CLIP_SECONDS);
    expect(clipLength(Number.NaN)).toBe(CLIP_SECONDS);
    expect(clipLength(90)).toBe(30);
  });
  it("goes back to the newest save point", () => {
    expect(latest([])).toBeUndefined();
    expect(latest([{ id: 1 }, { id: 3 }, { id: 2 }])).toBe(3);
  });
});

describe("the rebuilt game's input", () => {
  it("is the pad's bits, as the frames file the view builds from an input log", () => {
    expect(modelLine(0x81)).toBe("81");
    expect(modelLine(0x02, true)).toBe("02 r");
    // The same presses through the input log's letters.
    expect(framesScript(["P R......A", "- ......B.", "P! ......B."])).toBe(`gp-frames 1\n${modelLine(0x81)}\n${modelLine(0x02, true)}\n`);
  });
});

describe("the requests", () => {
  it("sends what the server expects", async () => {
    const calls: { url: string; init?: RequestInit }[] = [];
    const api = new Api("", (input, init) => {
      calls.push({ url: String(input), init });
      return Promise.resolve(new Response(JSON.stringify({ session: {}, branch: {}, save: {}, frames: 2 }), { status: 200, headers: { "Content-Type": "application/json" } }));
    });
    const c = new FreePlayClient(api);
    await c.start("hop", "walk", 60);
    await c.log("free-1", 1, 0, "P R.......");
    await c.save("free-1", { branch: 1, frame: 60, hash: "h", digest: "d" });
    await c.branch("free-1", 1);
    await c.startState("free-1", 2, 60);
    await c.clip("hop", "free-1/1", "gp-input-log 1\ncontrollers 1\nP ........\n");
    await c.clip("hop", "free-1/1", "gp-input-log 1\ncontrollers 1\nP ........\n", "clip.1.1");
    expect(calls.map((x) => [x.url, x.init?.method])).toEqual([
      ["/api/play/free", "POST"],
      ["/api/play/free/free-1/branches/1/log", "POST"],
      ["/api/play/free/free-1/saves", "POST"],
      ["/api/play/free/free-1/branches", "POST"],
      ["/api/play/free/free-1/start-state", "POST"],
      ["/api/capture/hop/scenarios/played", "POST"],
      ["/api/capture/hop/scenarios/played", "POST"],
    ]);
    const body = (i: number) => JSON.parse(String(calls[i].init?.body));
    expect(body(0)).toEqual({ run: "hop", recording: "walk", frame: 60 });
    expect(body(3)).toEqual({ from_save: 1 });
    expect(body(5)).toEqual({ save: "free-1/1", log: "gp-input-log 1\ncontrollers 1\nP ........\n" });
    expect(body(6).replace).toBe("clip.1.1");
    expect(c.picture("free-1", 2)).toBe("/api/play/free/free-1/saves/2/picture.png");
  });
});

describe("the panel", () => {
  it("says it's starting before the player loads", () => {
    const canvas = { current: null };
    const html = renderToString(<FreePlay api={new Api()} run="hop" recording="walk" frame={60} canvas={canvas} />);
    expect(html).toContain("Starting the original here.");
    expect(html).toContain("make a save point (F5)");
    expect(html).toContain('data-testid="clip-seconds"');
  });
});
