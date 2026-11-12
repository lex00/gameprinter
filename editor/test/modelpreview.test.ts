// The rebuilt game's messages (src/modelpreview.ts): the physics profile
// goes to the preview once it's ready, after its start, and again to a page
// that loads again.
import { describe, expect, it } from "vitest";
import { ModelPreview } from "../src/modelpreview";

const ORIGIN = "http://127.0.0.1:7700";
(globalThis as { location?: { origin: string } }).location = { origin: ORIGIN };

function fake() {
  const sent: unknown[] = [];
  const win = { postMessage: (m: unknown) => sent.push(m) };
  const frame = { contentWindow: win } as unknown as HTMLIFrameElement;
  const m = new ModelPreview(() => frame);
  const ready = () => m.receive({ source: win, origin: ORIGIN, data: { gp: "ready" } } as unknown as MessageEvent);
  return { m, sent, ready };
}

describe("the rebuilt game's physics", () => {
  it("sends the profile after the start, once the preview is ready", () => {
    const { m, sent, ready } = fake();
    m.profile("normal");
    m.loadState("AAAA", "gp-frames 1\n", 0);
    expect(sent).toEqual([]);
    ready();
    expect(sent).toEqual([
      { gp: "load-state", state: "AAAA", text: "gp-frames 1\n", frame: 0 },
      { gp: "profile", name: "normal" },
    ]);
  });

  it("sends a change at once, and nothing for the same profile", () => {
    const { m, sent, ready } = fake();
    ready();
    m.profile("retro");
    expect(sent).toEqual([]);
    m.profile("normal");
    m.profile("normal");
    expect(sent).toEqual([{ gp: "profile", name: "normal" }]);
    // A page loaded again starts in retro and is told again.
    m.reset();
    ready();
    expect(sent).toEqual([
      { gp: "profile", name: "normal" },
      { gp: "profile", name: "normal" },
    ]);
  });
});
