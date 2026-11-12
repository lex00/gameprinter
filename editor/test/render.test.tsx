// Every registered view renders, alone and in diff mode, from a pane built
// the way the shell builds one.
import { describe, expect, it } from "vitest";
import { renderToString } from "react-dom/server";
import { Api } from "../src/api";
import type { Pane } from "../src/pane";
import { registeredViews } from "../src/views/registry";
import "../src/views/index";
import { App } from "../src/App";

function pane(id: string, poke: number): Pane {
  const ram = new Uint8Array(2048);
  ram[0x10] = poke;
  const oam = new Uint8Array(256).fill(0xff);
  oam.set([100, 1, 0, 50], 0);
  return {
    id,
    name: `run ${id}`,
    frame: 5,
    show: { id, names: [], stale: false, recording: { frames: 10, checkpoint_interval: 60, has_trace: true, has_events: true, source_id: null, runtime: { name: "gp-nes", version: "2" } } },
    manifest: { frames: 10, input_log: "", checkpoint_interval: 60 },
    fields: [],
    labels: { id, game: "test", file: "labels.toml", missing: null, labels: [{ name: "X", addr: 0x10, size: 1, description: "x" }] },
    input: [],
    state: new Map([
      ["ram", ram],
      ["oam", oam],
      ["nametables", new Uint8Array(2048)],
      ["ppu.t", Uint8Array.from([0, 0])],
      ["mapper.mirroring", Uint8Array.from([0])],
    ]),
    prev: new Map([["ram", new Uint8Array(2048)]]),
    events: { id, frames: [1, 11], from: "event stream", events: [{ frame: 5, scanline: 241, dot: 3, kind: "nmi", addr: 0x8000, value: 0 }] },
  };
}

describe("views", () => {
  const api = new Api("", () => Promise.reject(new Error("no server in this test")));
  it("are registered once each", () => {
    const ids = registeredViews().map((v) => v.id);
    expect(new Set(ids).size).toBe(ids.length);
    expect(ids).toEqual(expect.arrayContaining(["ram-heatmap", "ram-watch", "oam", "nametables", "event-lanes", "diff"]));
  });
  for (const v of registeredViews()) {
    it(`${v.id} renders`, () => {
      const html = renderToString(<v.component api={api} frame={5} a={pane("aaaa", 1)} setFrame={() => {}} />);
      if (!v.diffOnly) expect(html.length).toBeGreaterThan(0);
      const both = renderToString(
        <v.component api={api} frame={5} a={pane("aaaa", 1)} b={pane("bbbb", 2)} diff={{ a: "aaaa", b: "bbbb", frame: 5, ram_differs: true, fields: [] }} setFrame={() => {}} />,
      );
      expect(both.length).toBeGreaterThan(0);
    });
  }
  it("the watch list shows a label's value", () => {
    const watch = registeredViews().find((v) => v.id === "ram-watch")!;
    expect(renderToString(<watch.component api={api} frame={5} a={pane("aaaa", 0x2a)} setFrame={() => {}} />)).toContain("$2a (42)");
  });
  it("the shell renders before anything loads", () => {
    expect(renderToString(<App api={api} />)).toContain("Trace inspector");
  });
});
