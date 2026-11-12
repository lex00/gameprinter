// "Play the rebuild" (docs/show.md): the header button, the playable page's
// address, the bar's words, and `level=` writing the start level into a bundle.
import { describe, expect, it } from "vitest";
import { renderToString } from "react-dom/server";
import { Api } from "../src/api";
import { CONTROLS, PlayRebuild, playUrl, profileName } from "../src/PlayRebuild";
import { Unified } from "../src/Unified";
// @ts-expect-error a plain browser module with no types
import { withLevel } from "../../scripts/web/page/level.js";

const api = new Api("");

describe("the mode switch", () => {
  it("is one header button, and the screen starts as the clips", () => {
    const html = renderToString(<Unified api={api} run="hop" />).replace(/<!-- -->/g, "");
    expect(html).toContain('data-testid="ctl-play-rebuild"');
    expect(html).toContain("play the rebuild");
    expect(html).toContain('data-testid="screen-original"');
    expect(html).not.toContain('data-testid="play-rebuild"');
    // Beside "record", in the header.
    const head = html.slice(html.indexOf('data-testid="controls"'), html.indexOf('data-testid="screen-original"'));
    expect(head.indexOf("ctl-freeplay")).toBeLessThan(head.indexOf("ctl-play-rebuild"));
  });
  it("shows the playable game, the bar, the keys and a way back instead of the clips", () => {
    const html = renderToString(<PlayRebuild api={api} run="hop" back={() => {}} />).replace(/<!-- -->/g, "");
    expect(html).toContain('data-testid="play-frame"');
    expect(html).toContain('src="/api/capture/hop/preview/index.html?files=here"');
    expect(html).not.toContain("control=1");
    expect(html).toContain('data-testid="play-back"');
    expect(html).toContain(CONTROLS);
  });
  it("says the keys in one line", () => {
    expect(CONTROLS).toBe("arrows move, Z jump, X run, Enter start, F9 widescreen, F10 dim flashes, 5-9 mute");
  });
});

describe("the playable page's address", () => {
  it("joins a preset and switches with +, and leaves retro and no level out", () => {
    expect(profileName("retro", [])).toBe("retro");
    expect(profileName("normal", [])).toBe("normal");
    expect(profileName("normal", ["no_timer"])).toBe("normal+no_timer");
    expect(profileName("retro", ["slow_slugs"])).toBe("slow_slugs");
    expect(playUrl(api, "hop", "retro")).toBe("/api/capture/hop/preview/index.html?files=here");
    expect(playUrl(api, "hop", "normal+no_timer", "w1_2")).toBe("/api/capture/hop/preview/index.html?profile=normal+no_timer&level=w1_2&files=here");
  });
});

describe("level=", () => {
  const bytes = (s: string) => new TextEncoder().encode(s);
  const text = (b: Uint8Array) => new TextDecoder().decode(b);
  it("replaces the start level the model already names", () => {
    const b = bytes('{"model":"[model]\\nname = \\"x\\"\\nstart_level = \\"w1_1\\"\\n"}');
    expect(text(withLevel(b, "w1_2"))).toBe('{"model":"[model]\\nname = \\"x\\"\\nstart_level = \\"w1_2\\"\\n"}');
  });
  it("adds one when the model has none, and leaves the bundle alone without a level", () => {
    const b = bytes('{"model":"[model]\\nname = \\"x\\"\\n"}');
    expect(text(withLevel(b, "w1_2"))).toBe('{"model":"[model]\\nstart_level = \\"w1_2\\"\\nname = \\"x\\"\\n"}');
    expect(withLevel(b, null)).toBe(b);
  });
  it("stays valid JSON the bundle's reader takes", () => {
    const out = JSON.parse(text(withLevel(bytes('{"model":"[model]\\nname = \\"x\\"\\n"}'), "w1_2")));
    expect(out.model).toContain('start_level = "w1_2"');
  });
});
