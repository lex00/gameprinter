// The play panel (docs/show.md): the buttons it reads, the pieces of input
// log it sends, its pacing, a session's events folded into marks and
// explanations, the requests it makes, and that it draws what it's given.
import { describe, expect, it } from "vitest";
import { renderToString } from "react-dom/server";
import { Api, type CaptureEvent } from "../src/api";
import { logLine } from "../src/livefold";
import { stubImports } from "../src/nesplayer";
import { PlayPanel, PlayReport } from "../src/Play";
import { NES_FPS, emptyPlay, foldPlay, frameAt, frameLines, framesDue, markBox, modelFrame, modelSteps, padFromGamepad, playStep, triggerPresses, padFromKeys, pressedWords, replayWindow, scriptFrom } from "../src/playfold";

let seq = 0;
const ev = (kind: string, data: Record<string, unknown>, actor: CaptureEvent["actor"] = "pipeline"): CaptureEvent => ({ v: 0, seq: ++seq, at_ms: seq, round: 1, actor, kind, data });

const session = [
  ev("play.start", { run: "play-smb1-1", name: "Super Mario Bros.", template: "smb1-ntsc-prg0", store: "/h/play/store", real_store: false }, "person"),
  ev("play.mark", { id: 1, from: 310, to: 400, note: "hit a brick", by: "key" }, "person"),
  ev("play.stop", { frames: 900, digest: "ab" }, "person"),
  ev("play.recorded", { id: "c0ffee00112233445566", frames: 900, digest_matches: true }),
  ev("play.annotated", { mark: 1, annotation: 4, recording: "c0ffee00112233445566" }),
  ev("show.start", { marks: [1] }),
  ev("show.mark", {
    mark: 1,
    summary: "background: 1 cell changes",
    missing: ["background: 1 cell changes in the original (45454747 -> 24242424 x1), none in the model", "counter score: the original +5 (0 -> 5), the model +0"],
    model: { frames: [280, 360], events: [["player.jumped (mario)", 1]] },
  }),
  ev("show.end", { ms: 2000, marks: 1, model_error: null }),
  ev("show.preview", { status: "skipped", why: "no lifted assets" }),
];

describe("reading the buttons", () => {
  it("maps keys to the input log's bits", () => {
    expect(padFromKeys(new Set())).toBe(0);
    expect(padFromKeys(new Set(["KeyX"]))).toBe(1);
    expect(padFromKeys(new Set(["ArrowRight", "KeyZ"]))).toBe(0x82);
    expect(padFromKeys(new Set(["Enter", "ArrowUp"]))).toBe(0x18);
  });
  it("reads a standard gamepad, the stick as the d-pad", () => {
    const pressed = (on: number[]) => Array.from({ length: 16 }, (_, i) => ({ pressed: on.includes(i) }));
    expect(padFromGamepad(null)).toBe(0);
    expect(padFromGamepad({ buttons: pressed([0, 9]), axes: [0, 0] })).toBe(1 | 8);
    expect(padFromGamepad({ buttons: pressed([2, 15]), axes: [0, 0] })).toBe(2 | 128);
    expect(padFromGamepad({ buttons: pressed([]), axes: [-0.9, 0.8] })).toBe(64 | 32);
  });
});

describe("pacing and the log", () => {
  it("runs one frame per NES frame time and never races after a pause", () => {
    const tick = 1000 / NES_FPS;
    expect(framesDue(0, tick * 0.5).frames).toBe(0);
    const a = framesDue(0, tick * 2.5);
    expect(a.frames).toBe(2);
    expect(a.acc).toBeCloseTo(tick * 0.5);
    expect(framesDue(0, 10_000).frames).toBe(4);
  });
  it("sends frame lines without the header, and aligns the model on polled frames", () => {
    const log = "gp-input-log 0\ncontrollers 1\nP R.......\n- R.......\nP .......A\n";
    const lines = frameLines(log);
    expect(lines).toEqual(["P R.......", "- R.......", "P .......A"]);
    expect(modelFrame(lines, 0)).toBe(0);
    expect(modelFrame(lines, 2)).toBe(1);
    expect(modelFrame(lines, 3)).toBe(2);
  });
  it("starts the view's model at the original's state and steps it on the polled frames after it, as a lockstep from there pairs them", () => {
    // A made-up log: frames 1 to 8, frame 4 a lag frame.
    const lines = ["P ........", "P R.......", "P R......A", "- R......A", "P R.....B.", "P .......A", "P ........", "P R......."];
    // A lockstep from frame 2: the model loaded at the original's frame 2
    // (model frame 1), then a pair per polled frame after it.
    const start = { original_frame: 2, frame: 1 };
    const pairs: [number, number][] = [[1, 2], [2, 3], [3, 5], [4, 6], [5, 7], [6, 8]];
    for (const [m, o] of pairs) expect(modelSteps(lines, start.original_frame, o)).toBe(m - start.frame);
    // Before the start the model stands at its start state.
    expect(modelSteps(lines, 2, 1)).toBe(0);
    // The script holds the presses from the start on, the lag frame left out.
    expect(scriptFrom(lines, 2)).toBe("gp-frames 1\n81\n82\n01\n00\n80\n");
  });
  it("plays a focus in a loop and stops both games where they drift apart", () => {
    const focus = { from: 100, to: 400 };
    expect(playStep(150, 2, { total: 1000, focus })).toEqual({ frame: 152, stop: false });
    expect(playStep(399, 2, { total: 1000, focus })).toEqual({ frame: 100, stop: false });
    // Crossing the drift frame stops there, even 3 seconds into a 5-second loop.
    expect(playStep(279, 2, { total: 1000, focus, stopAt: 280 })).toEqual({ frame: 280, stop: true });
    // Played on from it, it goes on.
    expect(playStep(280, 2, { total: 1000, focus, stopAt: 280 })).toEqual({ frame: 282, stop: false });
    expect(playStep(998, 4, { total: 1000 })).toEqual({ frame: 1000, stop: true });
  });
  it("names the buttons both games get on a frame in the model's action words", () => {
    const lines = ["P R.....B.", "P .......A", "P ........"];
    const actions = { jump: "a", run: "b", right: "right", left: "left" };
    expect(pressedWords(lines, 1, actions)).toBe("Right + B (run)");
    expect(pressedWords(lines, 2, actions)).toBe("A (jump)");
    expect(pressedWords(lines, 3, actions)).toBe("nothing");
    expect(pressedWords(lines, 1, null)).toBe("Right + B");
  });
  it("places marks on the timeline and replays a second around them", () => {
    expect(markBox({ from: 0, to: 99 }, 99)).toEqual({ left: 0, width: 100 });
    expect(frameAt(50, 100, 900)).toBe(450);
    expect(frameAt(-5, 100, 900)).toBe(0);
    expect(replayWindow({ from: 30, to: 880 }, 900)).toEqual({ from: 0, to: 900 });
    expect(replayWindow({ from: 300, to: 400 }, 900)).toEqual({ from: 240, to: 460 });
  });
});

describe("a session's events", () => {
  it("fold into marks, the recording and each mark's explanation", () => {
    const s = foldPlay(emptyPlay(), session);
    expect(s.run).toBe("play-smb1-1");
    expect(s.realStore).toBe(false);
    expect(s.marks).toEqual([{ id: 1, from: 310, to: 400, note: "hit a brick", by: "key", annotation: 4 }]);
    expect(s.recording).toBe("c0ffee00112233445566");
    expect(s.digestMatches).toBe(true);
    expect(s.explaining).toBe(false);
    expect(s.explanations[1].missing).toHaveLength(2);
    expect(s.preview).toEqual({ status: "skipped", why: "no lifted assets" });
    // Folding again changes nothing.
    expect(foldPlay(s, session)).toEqual(s);
  });
  it("read as log lines", () => {
    expect(logLine(session[1])).toBe("mark 1 on frames 310 to 400: hit a brick");
    expect(logLine(session[3])).toContain("hashes agree");
    expect(logLine(session[6])).toBe("mark 1: background: 1 cell changes");
  });
  it("draw the marks, what the original did and what the model did", () => {
    const html = renderToString(<PlayReport state={foldPlay(emptyPlay(), session)} frames={900} />);
    expect(html).toContain("every frame hash agrees");
    expect(html).toContain("in the scratch store");
    expect(html).toContain("&quot;hit a brick&quot;");
    expect(html).toContain("45454747 -&gt; 24242424");
    expect(html).toContain("player.jumped (mario) x1");
    expect(html).toMatch(/model preview: (<!-- -->)?skipped/);
  });
  it("say so when the frame hashes differ", () => {
    const bad = foldPlay(emptyPlay(), [ev("play.recorded", { id: "x", frames: 3, digest_matches: false })]);
    expect(renderToString(<PlayReport state={bad} frames={3} />)).toContain("FRAME HASHES DIFFER");
  });
});

describe("the panel", () => {
  it("starts with the ROM picker and the store choice, scratch by default", () => {
    const html = renderToString(<PlayPanel api={new Api()} />);
    expect(html).toContain("record into the real store");
    expect(html).not.toContain("checked");
    expect(html).toContain('data-testid="play-screen"');
  });
  it("sends what the server expects", async () => {
    const calls: { url: string; init?: RequestInit }[] = [];
    const api = new Api("", (input, init) => {
      calls.push({ url: String(input), init });
      return Promise.resolve(new Response(JSON.stringify({ session: {}, mark: {}, frames: 2 }), { status: 200, headers: { "Content-Type": "application/json" } }));
    });
    await api.playStart({ rom: "ab", notes: "n", real_store: true });
    await api.playLog("play-x-1", 0, "P R.......\n- R.......");
    await api.playMark("play-x-1", { from: 10, to: 20, note: "brick", by: "key" });
    await api.playStop("play-x-1", 2, "ff");
    expect(calls.map((c) => [c.url, c.init?.method])).toEqual([
      ["/api/play/sessions", "POST"],
      ["/api/play/sessions/play-x-1/log", "POST"],
      ["/api/play/sessions/play-x-1/marks", "POST"],
      ["/api/play/sessions/play-x-1/stop", "POST"],
    ]);
    expect(JSON.parse(String(calls[0].init?.body))).toEqual({ rom: "ab", notes: "n", real_store: true });
    expect(JSON.parse(String(calls[1].init?.body))).toEqual({ offset: 0, lines: "P R.......\n- R......." });
    expect(JSON.parse(String(calls[3].init?.body))).toEqual({ frames: 2, digest: "ff" });
  });
});

describe("the player module", () => {
  it("gets a throwing stub for every import", () => {
    // A module importing env.f, built by hand: magic, version, a type
    // section with () -> (), and an import section.
    const bytes = new Uint8Array([0, 0x61, 0x73, 0x6d, 1, 0, 0, 0, 1, 4, 1, 0x60, 0, 0, 2, 9, 1, 3, 0x65, 0x6e, 0x76, 1, 0x66, 0, 0]);
    const m = new WebAssembly.Module(bytes);
    const imports = stubImports(m) as Record<string, Record<string, () => void>>;
    expect(Object.keys(imports)).toEqual(["env"]);
    expect(() => imports.env.f()).toThrow(/never needs/);
    expect(new WebAssembly.Instance(m, imports)).toBeTruthy();
  });
});

describe("the controller's triggers", () => {
  const pad = (b: Record<number, { pressed: boolean; value?: number }>) => ({
    buttons: Array.from({ length: 16 }, (_, i) => b[i] ?? { pressed: false, value: 0 }),
  });
  it("save on the left trigger and load on the right, once a press", () => {
    let held = { save: false, load: false };
    const a = triggerPresses(pad({ 6: { pressed: true, value: 1 } }), held);
    expect([a.save, a.load]).toEqual([true, false]);
    held = a.held;
    const b = triggerPresses(pad({ 6: { pressed: true, value: 1 } }), held);
    expect([b.save, b.load]).toEqual([false, false]);
    const c = triggerPresses(pad({}), b.held);
    const d = triggerPresses(pad({ 7: { pressed: false, value: 0.8 } }), c.held);
    expect([d.save, d.load]).toEqual([false, true]);
  });
  it("read nothing with no gamepad", () => {
    expect(triggerPresses(null, { save: true, load: false })).toEqual({ save: false, load: false, held: { save: false, load: false } });
  });
});
