// The watch view (docs/watch.md): two sessions' event files folded into
// the PRG map's lit pages, the stops, the kept clips and the feed. The
// fixtures are sessions of Moss Hop and of its practice variant with the
// bump handler in bank 1 (`gp watch start NAME ROM`), so no commercial ROM
// data; the counts below were read from the files with jq, not from the
// fold.
import { describe, expect, it } from "vitest";
import { renderToString } from "react-dom/server";
import { Api } from "../src/api";
import { bankRows, emptyWatch, fold, parseLines, phrase } from "../src/watchfold";
import { WatchBody } from "../src/Watch";
import { inputLine, type WatchInput } from "../src/watchapi";
import bump from "./fixtures/watch-bump.ndjson?raw";
import mosshop from "./fixtures/watch-mosshop.ndjson?raw";

const files: Record<string, string> = { "watch-bump.ndjson": bump, "watch-mosshop.ndjson": mosshop };
const read = (f: string) => parseLines(files[f]);

describe("watchfold", () => {
  it("lights the pages Moss Hop's runs ran and keeps its clip", () => {
    const evs = read("watch-mosshop.ndjson");
    const s = fold(emptyWatch(), evs);
    expect(s.seq).toBe(evs.length);
    expect(Object.keys(s.pages).length).toBe(2);
    expect(s.codeBytes).toBe(1727);
    expect(s.edges).toBe(294);
    expect(s.stops).toEqual([]);
    expect(s.kept.length).toBe(1);
    expect(s.kept[0].endHash).toMatch(/^[0-9a-f]{64}$/);
    expect(s.ended).toBe(true);
    expect(s.read).toEqual({ read: 37, routines: 37 });
    expect(s.behaviour.length).toBe(16);
  });

  it("shows bank 1's first code before the stop it went past", () => {
    const evs = read("watch-bump.ndjson");
    const s = fold(emptyWatch(), evs);
    expect(Object.keys(s.pages).map(Number).sort((a, b) => a - b)).toEqual([16, 48, 49]);
    expect(s.codeBytes).toBe(1732);
    expect(s.edges).toBe(297);
    expect(s.banks.map((b) => [b.bank, b.at])).toEqual([[1, "01:8000"]]);
    expect(s.stops.length).toBe(1);
    const stop = s.stops[0];
    expect(stop.reached?.routine).toBe("sub_01_8000");
    expect(s.banks[0].seq).toBeLessThan(stop.reached!.seq);
    expect(s.banks[0].frame).toBe(stop.reached!.frame);
    expect(s.kept.length).toBe(2);
    expect(s.behaviour.length).toBe(17);
    // Bank 1's row has its one page lit; the map has four 16 KB banks.
    const rows = bankRows(s);
    expect(rows.length).toBe(4);
    expect(rows[1].pages.filter((p) => p.seen).length).toBe(1);
  });

  it("folds a stream in pieces as it folds the whole", () => {
    const evs = read("watch-bump.ndjson");
    let s = emptyWatch();
    for (let i = 0; i < evs.length; i += 37) s = fold(s, evs.slice(i, i + 37));
    // An event given twice counts once.
    s = fold(s, evs.slice(100, 140));
    expect(s).toEqual(fold(emptyWatch(), evs));
  });

  it("says what happened in words", () => {
    const evs = read("watch-bump.ndjson");
    const said = evs.map(phrase).filter(Boolean) as string[];
    expect(said).toContain("Bank 1 runs for the first time, at 01:8000 (frame 489)");
    expect(said.some((t) => t.startsWith("A run goes past where the reader stopped (followed, not read@C27D) into sub_01_8000"))).toBe(true);
  });
});

describe("the watch view", () => {
  it("draws the map, the stop and the feed", () => {
    const s = fold(emptyWatch(), read("watch-bump.ndjson"));
    const html = renderToString(<WatchBody api={new Api("", () => Promise.reject(new Error("no server")), undefined)} name="bump" state={s} />);
    expect(html).toContain('data-testid="watch-banks" data-lit="3"');
    expect(html).toContain('data-reached="yes"');
    expect(html).toContain("a run went past, into sub_01_8000 at frame 489");
    expect(html).toContain("Bank 1 runs for the first time");
    expect(html).toContain("Stage 0 reads 37 of 38 routines");
    expect(html).toContain("297 edges first taken");
    expect(html).toContain("background cell kind 00000000 to 20212223");
  });
});

describe("the agent and the person in a session", () => {
  const tool: WatchInput = { v: 1, actor: "agent", kind: "tool", data: { tool: "watch_hook", args: "{at: [01:8000]}", result: "{hook_hits: 133}" } };
  const point: WatchInput = {
    v: 1,
    actor: "person",
    kind: "point",
    data: { recipe: "ab", frame: 600, rect: [96, 120, 8, 8], answer: { words: ["sprite 0, written by sub_C623 (C643) at frame 600"] } },
  };

  it("says each input in words", () => {
    expect(inputLine(tool)).toBe("The agent called watch_hook: {hook_hits: 133}");
    expect(inputLine(point)).toBe("You pointed at frame 600: sprite 0, written by sub_C623 (C643) at frame 600");
    expect(inputLine({ ...point, data: { ...point.data, answer: { words: [] } } })).toBe("You pointed at frame 600: no sprite there");
  });

  it("lists the agent's calls and the points beside the feed, not the page's own views", () => {
    const s = fold(emptyWatch(), read("watch-bump.ndjson"));
    const view: WatchInput = { v: 1, actor: "person", kind: "view", data: { recipe: "ab", label: "Kept clip" } };
    const html = renderToString(<WatchBody api={new Api("", () => Promise.reject(new Error("no server")), undefined)} name="bump" state={s} inputs={[view, tool, point]} />);
    expect(html).toContain("The agent called watch_hook");
    expect(html).toContain("You pointed at frame 600");
    expect(html).not.toContain("The page shows Kept clip");
  });
});
