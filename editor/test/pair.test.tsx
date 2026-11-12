// The workspace's side-by-side, resolution card and change views: their
// arithmetic, the requests they send, and that they draw what they're given.
import { describe, expect, it } from "vitest";
import { renderToString } from "react-dom/server";
import { Api, type CardJson, type PairFrameJson, type PairJson, type VariantRun } from "../src/api";
import { getShared, setShared } from "../src/shared";
import { CardBody, checkText, factText } from "../src/views/Card";
import { differingFrames, differingInstances, instanceOf, modelFrameAt, nextDifference, originalFrameOf, styled } from "../src/views/pairing";
import { PairFrame } from "../src/views/Pair";
import { registeredViews } from "../src/views/registry";
import { Ghosts } from "../src/views/Variants";
import { editOf, ghostsAt, offsetFrom, parseValue, runnable, summary } from "../src/views/changes";
import "../src/views/index";

function recording(body: unknown) {
  const calls: string[] = [];
  const api = new Api("", (input) => {
    calls.push(String(input));
    return Promise.resolve(new Response(JSON.stringify(body), { status: 200, headers: { "Content-Type": "application/json" } }));
  });
  return { api, calls };
}

/** Server-rendered markup with React's text node separators removed. */
const plain = (html: string) => html.replace(/<!-- -->/g, "");

const rows = [
  { path: "entities.player[0].x", original: "40", model: "40", differs: false, note: null, ram: [16], labels: ["PLAYER_X"] },
  { path: "entities.slug[1].x", original: "100", model: "101", differs: true, note: null, ram: [32], labels: [] },
  { path: "rules.game.state", original: "playing", model: null, differs: true, note: null, ram: [], labels: [] },
];

describe("side by side", () => {
  it("names the entity a path belongs to", () => {
    expect(instanceOf("entities.slug[1].x")).toBe("entities.slug[1]");
    expect(instanceOf("rules.game.state")).toBeUndefined();
    expect(instanceOf("cameras.main.x")).toBeUndefined();
  });
  it("finds the entities with a differing path", () => {
    expect([...differingInstances(rows)]).toEqual(["entities.slug[1]"]);
    const boxes = [
      { path: "entities.slug[1]", kind: "slug", box: { x: 1, y: 2, w: 3, h: 4 } },
      { path: "entities.player[0]", kind: "player", box: { x: 1, y: 2, w: 3, h: 4 } },
    ];
    expect(styled(boxes, differingInstances(rows)).map((s) => s.differs)).toEqual([true, false]);
  });
  it("maps frames both ways and finds the next difference", () => {
    const pair = { model_frame: [0, 0, 0, 1, 1, 2, 3] };
    expect(modelFrameAt(pair, 4)).toBe(1);
    expect(modelFrameAt(pair, 99)).toBe(3);
    expect(modelFrameAt(undefined, 7)).toBe(7);
    expect(originalFrameOf(pair, 1)).toBe(3);
    expect(originalFrameOf(pair, 9)).toBeUndefined();
    const ranges: [number, number][] = [
      [10, 12],
      [40, 40],
    ];
    expect(differingFrames(ranges)).toBe(4);
    expect(nextDifference(ranges, 11)).toBe(40);
    expect(nextDifference(ranges, 40)).toBe(10);
    expect(nextDifference([], 0)).toBeUndefined();
  });
  it("asks the server for the pair and for a frame of it", async () => {
    const { api, calls } = recording({});
    await api.pair("aaaa", "bbbb");
    await api.pairFrame("aaaa", "bbbb", 60);
    expect(calls).toEqual(["/api/pair?original=aaaa&model=bbbb", "/api/pair/frame?original=aaaa&model=bbbb&frame=60"]);
  });
  it("draws both pictures and marks the paths that differ", () => {
    const data: PairFrameJson = {
      original: { id: "aaaa", frame: 60 },
      model: { id: "bbbb", frame: 56 },
      asked: 61,
      aligned: false,
      alignment: "polled",
      paths: rows,
      differing: 2,
      original_entities: [{ path: "entities.slug[1]", kind: "slug", box: { x: 10, y: 20, w: 8, h: 8 } }],
      model_entities: [{ path: "entities.slug[1]", kind: "slug", box: { x: 11, y: 20, w: 8, h: 8 } }],
      camera: null,
    };
    const html = plain(renderToString(<PairFrame data={data} />));
    expect(html).toContain("original, frame 60");
    expect(html).toContain("model, frame 56");
    expect(html).toContain("2 of 3 mapped paths differ");
    expect(html).toContain("Frame 61 is a lag frame");
    expect(html).toContain("not in the model");
    expect((html.match(/entity-box differs/g) ?? []).length).toBe(2);
    expect((html.match(/<tr class="differs"/g) ?? []).length).toBe(2);
  });
});

describe("resolution cards", () => {
  const card: CardJson = {
    id: "aaaa",
    frame: 60,
    path: "entities.slug[1]",
    part: "entities.slug",
    model: { name: "mosshop", dir: "templates/mosshop" },
    level: "metric",
    facts: [
      {
        path: "entities.slug.patrol.speed",
        value: "0.5",
        evidence: { entry: "entities.slug.patrol", source: "observed", confidence: 95, note: null, refs: [{ at: "baselines/mosshop/walk-and-turn.toml", in: null }] },
        level: "metric",
        rule: 0,
        tolerance: { speed: 0.03 },
      },
      { path: "entities.slug.hitbox.w", value: 14, evidence: null, level: "metric", rule: null, tolerance: null },
    ],
    facts_total: 2,
    values: [{ path: "entities.slug[1].x", value: "100", ram: [0x20], labels: ["SLUG1_X"] }],
    checks: [
      { part: "entities.slug.patrol", kind: "patrol", quantity: "speed", case: null, against: "original", level: "metric", status: "pass", samples: 4, expected: "0.5", actual: "0.5", tolerance: "3%", frame: null, detail: null, subject: "interp", validated_at: "" },
    ],
    results_for: "aaaa",
    links: [],
  };
  it("shows facts with evidence and level, values with RAM, and the measured numbers", () => {
    const html = plain(renderToString(<CardBody card={card} />));
    expect(html).toContain("contract level metric");
    expect(html).toContain("observed 95%");
    expect(html).toContain("baselines/mosshop/walk-and-turn.toml");
    expect(html).toContain("no evidence");
    expect(html).toContain("$0020");
    expect(html).toContain("SLUG1_X");
    expect(html).toContain("expected 0.5, measured 0.5, within 3%");
  });
  it("formats values and checks", () => {
    expect(factText("0.5")).toBe("0.5");
    expect(factText([1, 2])).toBe("[1,2]");
    expect(checkText({ ...card.checks[0], expected: null, actual: null, tolerance: null, detail: "no sample" })).toBe("no sample");
  });
  it("asks for a path at a frame, with results of another recording", async () => {
    const { api, calls } = recording({});
    await api.card("aaaa", "entities.slug[1]", 60, "bbbb");
    await api.facts("aaaa");
    expect(calls[0]).toBe("/api/recordings/aaaa/card?path=entities.slug%5B1%5D&frame=60&results=bbbb");
    expect(calls[1]).toBe("/api/recordings/aaaa/facts");
  });
  it("the shared path is what views set", () => {
    setShared({ cardPath: "entities.slug[0]" });
    expect(getShared().cardPath).toBe("entities.slug[0]");
    setShared({ changePath: "entities.player.jump.gravity" });
    expect(getShared()).toEqual({ cardPath: "entities.slug[0]", changePath: "entities.player.jump.gravity" });
  });
});

describe("describing a change", () => {
  const run = (name: string, dx: number): VariantRun => ({
    name,
    ok: true,
    frames: 3,
    first_difference: dx === 0 ? null : 1,
    camera: [[0, 0], [8, 0], [16, 0]],
    entities: [{ path: "entities.player[0]", kind: "player", hitbox: [2, 4, 12, 12], x: [10, 20 + dx, 30 + dx], y: [50, 50, 40], live: [1, 1, 0] }],
  });
  it("reads typed values as a model holds them", () => {
    expect(parseValue("3")).toBe(3);
    expect(parseValue("-2")).toBe(-2);
    expect(parseValue("0.125")).toBe("0.125");
    expect(parseValue("true")).toBe(true);
    expect(parseValue('"run"')).toBe("run");
    expect(parseValue("  ")).toBeUndefined();
    expect(editOf("entities.player.jump.gravity", "0.125")).toEqual({ path: "entities.player.jump.gravity", value: "0.125" });
    expect(editOf("", "1")).toBeUndefined();
    expect(editOf("a.b", "")).toBeUndefined();
  });
  it("runs only variants that change something", () => {
    expect(runnable([{ name: "a", edits: [] }, { name: "b", edits: [{ path: "x", value: 1 }] }]).map((v) => v.name)).toEqual(["b"]);
  });
  it("puts a run's live entities on the picture, less the camera", () => {
    expect(ghostsAt(run("a", 0), 1)).toEqual([{ path: "entities.player[0]", kind: "player", x: 20 + 2 - 8, y: 54, w: 12, h: 12 }]);
    expect(ghostsAt(run("a", 0), 2)).toEqual([]);
    expect(ghostsAt(run("a", 0), 5)).toEqual([]);
    expect(ghostsAt({ name: "bad", ok: false, error: "no" }, 0)).toEqual([]);
  });
  it("measures a ghost's offset from the model's", () => {
    expect(offsetFrom(run("m", 0), run("v", 5), "entities.player[0]", 1)).toEqual({ dx: 5, dy: 0 });
    expect(offsetFrom(run("m", 0), run("v", 5), "entities.nobody[0]", 1)).toBeUndefined();
  });
  it("says how a run parts from the model", () => {
    expect(summary(run("a", 0))).toContain("the same as the model");
    expect(summary(run("a", 3))).toBe("parts from the model at model frame 1");
    expect(summary({ name: "x", ok: false, error: "bad path" })).toBe("bad path");
  });
  it("sends the variants as JSON in the query", async () => {
    const { api, calls } = recording({});
    await api.variants("mmmm", [{ name: "floaty", edits: [{ path: "entities.player.jump.gravity", value: "0.125" }] }]);
    const url = new URL(calls[0], "http://x");
    expect(url.pathname).toBe("/api/recordings/mmmm/variants");
    expect(JSON.parse(url.searchParams.get("variants")!)).toEqual([{ name: "floaty", edits: [{ path: "entities.player.jump.gravity", value: "0.125" }] }]);
  });
  it("draws a translucent box for each live entity of each shown run", () => {
    const runs = [run("model", 0), run("floaty", 4)];
    const html = renderToString(
      <svg>
        <Ghosts runs={runs} modelFrame={1} />
      </svg>,
    );
    expect((html.match(/data-testid="ghost"/g) ?? []).length).toBe(2);
    const hidden = renderToString(
      <svg>
        <Ghosts runs={runs} modelFrame={1} hidden={new Set(["floaty"])} />
      </svg>,
    );
    expect((hidden.match(/data-testid="ghost"/g) ?? []).length).toBe(1);
  });
});

describe("the new views", () => {
  it("are registered", () => {
    const ids = registeredViews().map((v) => v.id);
    expect(ids).toEqual(expect.arrayContaining(["pair", "card", "variants"]));
  });
  it("say what to pick before a model run is open", () => {
    const api = new Api("", () => Promise.reject(new Error("no server")));
    const pane = { id: "aaaa", name: "x", frame: 0, show: {}, manifest: { frames: 1 }, fields: [], labels: {}, input: [] } as never;
    for (const id of ["pair", "variants"]) {
      const v = registeredViews().find((x) => x.id === id)!;
      expect(renderToString(<v.component api={api} frame={0} a={pane} setFrame={() => {}} />)).toContain("model run");
    }
  });
});

describe("pair JSON", () => {
  it("has the shape the views read", () => {
    const p: PairJson = { original: { id: "a", name: "o", frames: 9 }, model: { id: "b", name: "m", frames: 4 }, game: "g", mapping: "m.toml", alignment: "polled", input_matches: true, first_input_mismatch: null, pairs: 5, model_frame: [0, 1], paths: [], differing: [] };
    expect(p.alignment).toBe("polled");
  });
});
