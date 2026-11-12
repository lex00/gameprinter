// The themes view's helpers, its API calls and its readability table.
import { describe, expect, it } from "vitest";
import { renderToString } from "react-dom/server";
import { Api, type Layer, type Readability, type ThemeDoc } from "../src/api";
import { ReadabilityTable } from "../src/views/Themes";
import { addLayer, blankTheme, copyTheme, effectOf, moveLayer, ordered, ruleFor, setEffect, setRule, toggleLayer } from "../src/views/themeedit";

const stack: Layer[] = [
  { theme: "mine", scope: "personal" },
  { theme: "dusk", scope: "baked" },
  { theme: "paper", scope: "baked", enabled: false },
];

describe("stack editing", () => {
  it("orders baked under personal and moves only within a scope", () => {
    const s = ordered(stack);
    expect(s.map((l) => l.theme)).toEqual(["dusk", "paper", "mine"]);
    expect(moveLayer(s, 1, -1).map((l) => l.theme)).toEqual(["paper", "dusk", "mine"]);
    expect(moveLayer(s, 1, 1)).toBe(s);
    expect(toggleLayer(s, 1)[1].enabled).toBe(true);
    expect(toggleLayer(s, 0)[0].enabled).toBe(false);
    expect(addLayer(s, "dusk")).toBe(s);
    expect(addLayer(s, "night").at(-1)).toEqual({ theme: "night", scope: "personal" });
  });
});

describe("live effect editing", () => {
  const t: ThemeDoc = blankTheme("night", "Night", "mosshop");
  it("sets and clears a rule's fields", () => {
    const a = setRule(t, "player", "lightness", "1.2");
    expect(ruleFor(a, "player")).toEqual({ target: "player", lightness: 1.2 });
    const b = setRule(a, "player", "tint", "#ff0000");
    expect(ruleFor(b, "player").tint).toBe("#ff0000");
    expect(setRule(b, "player", "hue", "x")).toBe(b);
    const c = setRule(setRule(b, "player", "tint", ""), "player", "lightness", "");
    expect(c.palette).toEqual([]);
  });
  it("puts effects in place by kind and target", () => {
    const a = setEffect(t, "vignette", undefined, { kind: "vignette", strength: 0.4 });
    const b = setEffect(a, "vignette", undefined, { kind: "vignette", strength: 0.6 });
    expect(b.effect).toEqual([{ kind: "vignette", strength: 0.6 }]);
    const g = setEffect(b, "glow", "player", { kind: "glow", target: "player", color: "#fff000" });
    expect(effectOf(g, "glow", "player")).toBeDefined();
    expect(effectOf(g, "glow", "enemy")).toBeUndefined();
    expect(setEffect(g, "glow", "player", undefined).effect).toHaveLength(1);
  });
  it("a personal copy drops the restyles, whose files stay with the original", () => {
    const c = copyTheme({ ...t, restyle: [{}] }, "mine");
    expect(c.id).toBe("mine");
    expect(c.restyle).toBeUndefined();
  });
});

describe("theme requests", () => {
  it("previews and saves through gp serve's theme endpoints", async () => {
    const calls: { url: string; init?: RequestInit }[] = [];
    const api = new Api("", (input, init) => {
      calls.push({ url: String(input), init });
      return Promise.resolve(new Response("{}", { status: 200, headers: { "Content-Type": "application/json" } }));
    });
    await api.themes("mosshop");
    await api.themePreview("mosshop", { layers: stack, baseline: "stomp-then-pit", frames: [60] });
    await api.saveTheme("mosshop", { layers: [stack[0]] });
    expect(calls.map((c) => [c.url, c.init?.method ?? "GET"])).toEqual([
      ["/api/themes/mosshop", "GET"],
      ["/api/themes/mosshop/preview", "POST"],
      ["/api/themes/mosshop/save", "POST"],
    ]);
    expect(JSON.parse(String(calls[1].init?.body)).frames).toEqual([60]);
  });
});

describe("readability table", () => {
  it("shows each group's verdict and what's absent", () => {
    const r: Readability = {
      verdict: "refuse",
      background: ["#3ca0fa"],
      groups: [{ group: "player", pixels: 100, distinct: 0.1, contrast: 1.0, verdict: "refuse", why: "only 10% stand out" }],
      pairs: [],
      absent: ["text"],
    };
    const html = renderToString(<ReadabilityTable r={r} />).replaceAll("<!-- -->", "");
    expect(html).toContain("refuse");
    expect(html).toContain("10% distinct");
    expect(html).toContain("none in the model: text");
  });
});
