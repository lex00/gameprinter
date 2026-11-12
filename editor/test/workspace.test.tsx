// The workspace's additions to the inspector: box selection, lifted tiles
// in the game's colors, and annotations on the timeline.
import { describe, expect, it } from "vitest";
import { renderToString } from "react-dom/server";
import { Api, type Note, type SelectJson } from "../src/api";
import { marks, notesAt, parseTags } from "../src/annotate";
import { Timeline } from "../src/Annotations";
import { boxText, dragBox, parseBox, toPicture } from "../src/views/boxsel";
import { SelectionResult } from "../src/views/Select";
import { paletteChoices, parseColors, tileAt, tileRgba } from "../src/views/tilecolors";

/** An Api whose fetch records each request and answers with `body`. */
function recording(body: unknown) {
  const calls: { url: string; init?: RequestInit }[] = [];
  const api = new Api("", (input, init) => {
    calls.push({ url: String(input), init });
    return Promise.resolve(new Response(JSON.stringify(body), { status: 200, headers: { "Content-Type": "application/json" } }));
  });
  return { api, calls };
}

describe("box selection", () => {
  it("a drag is a box whichever way it goes, inside the picture", () => {
    expect(dragBox({ x: 10, y: 20 }, { x: 30, y: 25 })).toEqual({ x: 10, y: 20, w: 21, h: 6 });
    expect(dragBox({ x: 30, y: 25 }, { x: 10, y: 20 })).toEqual({ x: 10, y: 20, w: 21, h: 6 });
    expect(dragBox({ x: 5, y: 5 }, { x: 5, y: 5 })).toEqual({ x: 5, y: 5, w: 1, h: 1 });
    expect(dragBox({ x: -4, y: 230 }, { x: 300, y: 400 })).toEqual({ x: 0, y: 230, w: 256, h: 10 });
  });
  it("pointer positions scale to picture pixels", () => {
    const bounds = { left: 100, top: 50, width: 512, height: 480 };
    expect(toPicture(100, 50, bounds)).toEqual({ x: 0, y: 0 });
    expect(toPicture(100 + 511, 50 + 479, bounds)).toEqual({ x: 255, y: 239 });
    expect(toPicture(100 + 64, 50 + 100, bounds)).toEqual({ x: 32, y: 50 });
    expect(toPicture(0, 0, bounds)).toEqual({ x: 0, y: 0 });
  });
  it("boxes round-trip as text", () => {
    const b = { x: 1, y: 2, w: 3, h: 4 };
    expect(parseBox(boxText(b))).toEqual(b);
    expect(parseBox("250,0,20,1")).toEqual({ x: 250, y: 0, w: 6, h: 1 });
    for (const bad of ["", "1,2,3", "a,b,c,d", "0,0,0,1", "256,0,1,1"]) expect(parseBox(bad)).toBeUndefined();
  });
  it("asks the server for the box at the frame", async () => {
    const { api, calls } = recording({});
    await api.select("abcd", 42, { x: 1, y: 2, w: 3, h: 4 });
    expect(calls[0].url).toBe("/api/recordings/abcd/select?frame=42&rect=1,2,3,4");
  });
  it("lists the entities' model paths, the RAM under them with labels, and sprites", () => {
    const result: SelectJson = {
      id: "abcd",
      frame: 7,
      from: "trace",
      game: "mosshop",
      mapping: "templates/mosshop/mapping.toml",
      missing: null,
      rect: { x: 10, y: 160, w: 20, h: 20 },
      camera: { path: "cameras.main", x: 64, y: null },
      world: { x: 74, y: 160, w: 20, h: 20 },
      sprites: [{ index: 3, oam: 12, box: { x: 12, y: 170, w: 8, h: 8 }, tile: 0x14, attr: 1 }],
      entities: [
        {
          id: 1,
          path: "entities.slug[0]",
          kind: "slug",
          box: { x: 12, y: null, w: 12, h: null },
          fields: [{ path: "entities.slug[0].x", value: "76", ram: [0x20, 0x22], note: "ENEMY_X, ENEMY_X_HI" }],
        },
      ],
      ram: [{ addr: 0x20, value: 76, labels: ["ENEMY_X"], paths: ["entities.slug[0].x"] }],
    };
    const html = renderToString(<SelectionResult result={result} />);
    expect(html).toContain("entities.slug[0].x");
    expect(html).toContain("ENEMY_X");
    expect(html).toContain("$0020");
    expect(html).toContain("y not mapped");
    expect(html).toContain("sprite 3");
    expect(html).toContain("cameras.main");
    const none = renderToString(<SelectionResult result={{ ...result, entities: [], ram: [], sprites: [], missing: "no model under templates/ has a state mapping for x" }} />);
    expect(none).toContain("no model under templates/");
    expect(none).toContain("No sprite");
  });
});

describe("tiles in the game's colors", () => {
  const colors = parseColors(Array.from({ length: 64 }, (_, i) => [i, i + 1, i + 2].map((v) => v.toString(16).padStart(2, "0")).join("")));
  it("reads the 64 colors", () => {
    expect(colors).toHaveLength(64);
    expect(colors[0x10]).toEqual([0x10, 0x11, 0x12]);
  });
  it("offers palette RAM with the shared backdrop, then lifted palettes", () => {
    const ram = Uint8Array.from({ length: 32 }, (_, i) => i | 0x40);
    const choices = paletteChoices(ram, [{ name: "palettes", palettes: [[0x0f, 1, 2, 3]] }]);
    expect(choices).toHaveLength(9);
    expect(choices[0].colors).toEqual([0, 1, 2, 3]);
    expect(choices[5].colors).toEqual([0, 21, 22, 23]);
    expect(choices[5].label).toBe("this frame: sprites 1");
    expect(choices[8]).toEqual({ key: "lifted:palettes:0", label: "palettes 0", colors: [0x0f, 1, 2, 3] });
    expect(paletteChoices(undefined, [])).toEqual([]);
  });
  it("draws each index in its palette color and leaves the indices alone", () => {
    const pixels = "0123";
    const rgba = tileRgba(pixels, 2, 2, [0x0f, 0x10, 0x20, 0x30], colors);
    expect(Array.from(rgba.slice(0, 4))).toEqual([0x0f, 0x10, 0x11, 255]);
    expect(Array.from(rgba.slice(4, 8))).toEqual([0x10, 0x11, 0x12, 255]);
    expect(Array.from(rgba.slice(12, 16))).toEqual([0x30, 0x31, 0x32, 255]);
    expect(pixels).toBe("0123");
  });
  it("finds the tile under a point", () => {
    expect(tileAt(0, 0, 16, 512)).toBe(0);
    expect(tileAt(17, 9, 16, 512)).toBe(18);
    expect(tileAt(8, 8, 1, 4)).toBeUndefined();
    expect(tileAt(0, 40, 2, 9)).toBeUndefined();
  });
});

describe("annotations", () => {
  const note = (id: number, from: number, to: number): Note => ({ id, from, to, text: `note ${id}`, created_at: "t" });
  it("marks sit over their frames, in rows when they overlap", () => {
    const ms = marks([note(1, 0, 49), note(2, 25, 30), note(3, 50, 99)], 99);
    expect(ms.map((m) => [m.id, m.row])).toEqual([
      [1, 0],
      [2, 1],
      [3, 0],
    ]);
    expect(ms[0].left).toBe(0);
    expect(ms[0].width).toBe(50);
    expect(ms[2].left).toBe(50);
    expect(ms[1].title).toBe("frames 25 to 30: note 2");
    expect(marks([note(4, 7, 7)], 10000)[0].width).toBeGreaterThan(0.3);
  });
  it("finds the notes covering a frame", () => {
    const ns = [note(1, 0, 10), note(2, 5, 5)];
    expect(notesAt(ns, 5).map((n) => n.id)).toEqual([1, 2]);
    expect(notesAt(ns, 11)).toEqual([]);
  });
  it("reads tags from text", () => {
    expect(parseTags(" Bug, jump  bug,")).toEqual(["bug", "jump"]);
  });
  it("writes notes as JSON", async () => {
    const { api, calls } = recording({ annotation: note(1, 0, 0) });
    await api.addNote("abcd", { from: 1, to: 2, text: "x", tags: ["a"] });
    await api.updateNote("abcd", 3, { from: 1, to: 2, text: "y" });
    await api.deleteNote("abcd", 3);
    expect(calls.map((c) => [c.init?.method, c.url])).toEqual([
      ["POST", "/api/recordings/abcd/annotations"],
      ["PUT", "/api/recordings/abcd/annotations/3"],
      ["DELETE", "/api/recordings/abcd/annotations/3"],
    ]);
    expect(calls[0].init?.headers).toEqual({ "Content-Type": "application/json" });
    expect(JSON.parse(String(calls[0].init?.body))).toEqual({ from: 1, to: 2, text: "x", tags: ["a"] });
    expect(calls[2].init?.body).toBeUndefined();
  });
  it("the timeline draws a mark per note", () => {
    const html = renderToString(<Timeline notes={[note(1, 0, 10), note(2, 20, 20)]} frames={40} frame={5} setFrame={() => {}} />);
    expect(html.match(/timeline-mark/g)).toHaveLength(2);
    expect(html).toContain("frame 20: note 2");
  });
});
