// The game view (docs/inspector.md, "The game view"): sentences in game
// terms filled from a model's own words, the questions as pictures, what
// the model misses, the numbers with meanings, the explainer, and the
// layout that puts the two games first and the raw data in the details.
// The games are made up (and Moss Hop's words), so nothing here is one
// game's text.
import { describe, expect, it } from "vitest";
import { renderToString } from "react-dom/server";
import { Api, type CaptureEvent, type GameWords } from "../src/api";
import { App } from "../src/App";
import { Explainer, GameView, clipAround, startAt } from "../src/GameView";
import { emptyLive, fold } from "../src/livefold";
import {
  EXPLAINER,
  EXPLAINER_PLAYING,
  checkSentence,
  eventWords,
  failingChecks,
  gapGroups,
  headline,
  momentSentence,
  pathPhrase,
  plainQuestion,
  player,
  statusLine,
  thing,
  words,
} from "../src/phrase";
import { emptyPlay, foldPlay, framesScript } from "../src/playfold";

const pit = { name: "platformer.pit", about: "A player whose top edge reaches kill_below dies", class: "hazard", params: [{ name: "kill_below", type: "fixed", unit: "px" }], emits: ["player.died"] };
const hop: GameWords = {
  run: "hop",
  kind: "capture",
  vocab: {
    name: "hop",
    title: "Frog Pond",
    view: "side",
    entities: [
      { name: "frog", role: "player", description: "A green frog" },
      { name: "heron", role: "enemy", description: null },
    ],
    levels: [{ name: "pond", description: "One screen", size: [16, 15], tile_size: 16 }],
    counters: ["flies", "lives"],
    rules: { game: ["playing", "dying"] },
    actions: { jump: "a" },
  },
  mechanics: {
    mechanics: [pit, { name: "platformer.patrol", about: "Pacing back and forth", class: "mover", params: [{ name: "period", unit: "frames" }] }],
    paths: [
      { path: "levels.pond.kill_below", class: "parameter", mechanic: "platformer.pit" },
      { path: "entities.heron.patrol.period", class: "parameter", mechanic: "platformer.patrol" },
    ],
  },
  recordings: [
    { id: "aa", name: "walk-and-fall", frames: 400, input: "walk-and-fall" },
    { id: "bb", name: "catch-flies", frames: 300, input: "catch-flies" },
  ],
  rom: true,
  preview: true,
};

let seq = 0;
const ev = (round: number, kind: string, stage?: string, data?: Record<string, unknown>): CaptureEvent => ({ v: 1, seq: ++seq, at_ms: seq, round, actor: "pipeline", kind, stage, data });

const question = {
  round: 1,
  id: "derive:levels.pond.kill_below",
  text: "levels.pond.kill_below is 200 in the model, and pond.s:37 reads 208",
  path: "levels.pond.kill_below",
  candidates: [
    { from: "the model", value: "200" },
    { from: "gp model derive --from evidence", value: "208" },
  ],
};

const round1 = [
  ev(1, "round.start"),
  ev(1, "question", "questions", question),
  ev(1, "item", "gaps", { what: "gap", id: "ram:TMP_X", status: "open", text: "ram", data: { place: "ram", run: 0, samples: [5, 9] } }),
  ev(1, "item", "gaps", { what: "gap", id: "ram:TMP_Y", status: "open", text: "ram", data: { place: "ram", run: 0, samples: [6] } }),
  ev(1, "item", "gaps", { what: "gap", id: "oam:pal1 tiles 4 5", status: "open", text: "oam", data: { place: "oam", run: 1, samples: [40], size: [16, 16] } }),
  ev(1, "item", "validate", {
    what: "check",
    id: "rules.game state",
    status: "fail",
    text: "exact",
    data: { part: "rules.game", quantity: "state", kind: "machine", against: "original", expected: "playing", actual: "dying", frame: 272, from: "walk-and-fall" },
  }),
  ev(1, "item", "validate", {
    what: "check",
    id: "counters.lives value",
    status: "fail",
    text: "exact",
    data: { part: "counters.lives", quantity: "value", kind: "counter", against: "original", expected: "3", actual: "2", frame: 272, from: "walk-and-fall" },
  }),
  ev(1, "stage.end", "validate", { status: "fail", ms: 10, summary: "19 pass, 2 fail" }),
  ev(1, "metric", "validate", { key: "fidelity.interp.passed", value: 19, prev: null }),
  ev(1, "metric", "validate", { key: "fidelity.interp.failed", value: 2, prev: null }),
  ev(1, "metric", "derive", { key: "facts.derived_differ", value: 1, prev: null }),
  ev(1, "round.end", undefined, { ms: 30, failed: 1, deltas: {} }),
];

describe("words from the model", () => {
  it("names things the way the model does", () => {
    expect(words("cheep_cheep-red")).toBe("cheep cheep red");
    expect(player(hop)).toBe("Frog");
    expect(thing(hop, "heron")).toBe("the heron");
    expect(player({ ...hop, vocab: { ...hop.vocab, entities: [{ name: "player", role: "player", description: null }] } })).toBe("the player");
    expect(pathPhrase(hop, "entities.heron.patrol.period")).toBe("the heron's patrol period");
    expect(pathPhrase(hop, "counters.flies")).toBe("the flies count");
    expect(eventWords(hop, "player.died (frog)")).toBe("Frog died");
    expect(eventWords(hop, "enemy.hurt_player (heron)")).toBe("the heron hurt Frog");
  });

  it("asks a question in game terms from the mechanic, with each answer's picture line", () => {
    const q = plainQuestion(hop, question);
    expect(q.ask).toBe("Where does Frog fall out of the level?");
    expect(q.about).toBe("A player whose top edge reaches the line dies");
    expect(q.picture).toBe("row");
    expect(q.options.map((o) => [o.label, o.from, o.number])).toEqual([
      ["200 px", "what the model has now", 200],
      ["208 px", "what the game's code says", 208],
    ]);
  });

  it("asks from the unit when no template covers a parameter", () => {
    const q = plainQuestion({ ...hop, mechanics: { ...hop.mechanics, mechanics: [{ ...pit, name: "other.depth" }], paths: [{ path: "levels.pond.depth", class: "parameter", mechanic: "other.depth" }] } }, {
      ...question,
      id: "derive:levels.pond.depth",
      path: "levels.pond.depth",
    });
    expect(q.ask).toBe("What is the level's depth?");
  });

  it("says where the two games part and what to answer", () => {
    const s = fold(emptyLive(), round1);
    expect(failingChecks(s)).toHaveLength(2);
    expect(checkSentence(hop, failingChecks(s)[0])).toBe("the model is dying while the original is playing");
    expect(checkSentence(hop, failingChecks(s)[1])).toBe("the lives count is 2 in the model and 3 in the original");
    const line = statusLine(hop, s);
    expect(line).toContain("2 checks fail: the model is dying while the original is playing and the lives count is 2 in the model and 3 in the original at frame 272 of \"walk and fall\".");
    expect(line).toContain("A question for you: Where does Frog fall out of the level?");
    expect(line).not.toContain("kill_below");
  });

  it("drops the failing checks once validation passes", () => {
    const s = fold(emptyLive(), [...round1, ev(2, "stage.end", "validate", { status: "ok", ms: 9, summary: "21 pass" })]);
    expect(failingChecks(s)).toEqual([]);
  });

  it("groups gaps into a sentence per place, with a moment to show", () => {
    const g = gapGroups(Object.values(fold(emptyLive(), round1).found).filter((f) => f.what === "gap"));
    expect(g.map((x) => x.sentence)).toEqual(["The original keeps 2 values in its memory that the model has no name for", "The original draws a 16 by 16 sprite the model has nothing for"]);
    expect(g[1].frame).toBe(40);
    expect(g[1].recording).toBe(1);
  });

  it("explains a marked moment as a sentence: the original, then the model", () => {
    const x = {
      mark: 1,
      summary: "",
      missing: ["counter flies: the original +1"],
      original: {
        counters: [
          { name: "flies", before: 0, after: 1 },
          { name: "time", before: 9, after: 8 },
        ],
        cells: [{ before: "01", after: "02", count: 1 }],
        interactions: [{ interaction: { requests: ["Sfx_Gulp"], spawned: ["sprite pal2 tiles 60"] } }],
      },
      model: { counters: [{ name: "time", before: 9, after: 8 }], events: [["player.jumped (frog)", 2]] as [string, number][], tiles_changed: 0 },
    };
    const m = momentSentence(hop, x, "eat a fly");
    expect(m.sentence).toBe("Eat a fly: the original: flies +1, a piece of the background changes, a sound plays and something new appears; the model: Frog jumped (2 times).");
    expect(m.sentence).not.toContain("Sfx_");
  });

  it("gives a few numbers, each with its meaning", () => {
    const h = headline(fold(emptyLive(), round1).metrics);
    expect(h.map((n) => n.label)).toEqual(["checks pass", "checks fail"]);
    expect(h[1].means).toMatch(/differ/);
  });

  it("turns an input log into the model's frames: polled frames, the buttons as bits", () => {
    expect(framesScript(["- ........", "P R......A", "P! ....T...", "P ........"])).toBe("gp-frames 1\n81\n08 r\n00\n");
  });
});

describe("the game view", () => {
  it("starts at the moment the games part, and plays clips around a moment", () => {
    const s = fold(emptyLive(), round1);
    expect(startAt(hop, s)).toEqual({ recording: "walk-and-fall", frame: 212 });
    expect(startAt(hop, emptyLive())).toEqual({ recording: "walk-and-fall", frame: 1 });
    expect(clipAround(100, 120, 400)).toEqual({ from: 40, to: 150 });
  });

  it("draws the two games first, the status, the questions as pictures, what's missing and the numbers", () => {
    const s = fold(emptyLive(), round1);
    const html = renderToString(<GameView api={new Api("")} run="hop" words={hop} live={s} send={() => {}} />).replace(/<!-- -->/g, "");
    const at = (t: string) => html.indexOf(t);
    // Nothing above the two games: the status comes after them.
    expect(html.slice(at('data-testid="game-view"'), at('class="games '))).toBe('data-testid="game-view"><div ');
    expect(at('data-testid="game-status"')).toBeGreaterThan(at('data-testid="game-model"'));
    expect(html).toContain('src="/api/capture/hop/preview/index.html?control=1&amp;files=here"');
    expect(html).toContain("Where does Frog fall out of the level?");
    expect(html).toContain("208 px");
    expect(html).toContain("what the game&#x27;s code says");
    expect(html).toContain("The original draws a 16 by 16 sprite the model has nothing for");
    expect(html).toContain("checks pass");
    expect(html).toContain('data-testid="before-after"');
    // No model paths, labels or metric names in what a person reads first
    // (a number's metric name is only its tooltip).
    const read = html.replace(/ title="[^"]*"/g, "");
    for (const raw of ["levels.pond.kill_below", "TMP_X", "fidelity.interp", "derive:"]) expect(read).not.toContain(raw);
  });

  it("shows a play session's marks as sentences with the two games before and after", () => {
    let n = 100;
    const pev = (kind: string, data: Record<string, unknown>): CaptureEvent => ({ v: 1, seq: ++n, at_ms: n, round: 1, actor: "pipeline", kind, data });
    const play = foldPlay(emptyPlay(), [
      pev("play.mark", { id: 1, from: 300, to: 340, note: "eat a fly", by: "key" }),
      pev("show.mark", { mark: 1, from: 300, to: 340, summary: "", missing: ["x"], original: { counters: [{ name: "flies", before: 0, after: 1 }] }, model: { events: [] } }),
    ]);
    const words: GameWords = { ...hop, kind: "play", recordings: [{ id: "cc", name: "session", frames: 900, input: "session" }] };
    expect(startAt(words, emptyLive(), play)).toEqual({ recording: "session", frame: 240 });
    const html = renderToString(<GameView api={new Api("")} run="play-1" words={words} live={emptyLive()} play={play} send={() => {}} />).replace(/<!-- -->/g, "");
    expect(html).toContain("Eat a fly: the original: flies +1; the model does nothing.");
    expect(html).toContain("the model, after");
    expect(html).toContain("watch it");
  });

  it("explains each area in a line, and can be closed", () => {
    const html = renderToString(<Explainer open onClose={() => {}} />);
    expect(html).toContain("What am I looking at?");
    for (const e of EXPLAINER) expect(html).toContain(e.area);
    expect(renderToString(<Explainer open={false} onClose={() => {}} />)).toBe("");
    const playing = renderToString(<Explainer open playing onClose={() => {}} />);
    for (const e of EXPLAINER_PLAYING) expect(playing).toContain(e.area);
    expect(playing).not.toContain("The two pictures");
  });

  it("keeps the inspector one click away in the details, open when no game is shown", () => {
    const api = new Api("", () => Promise.resolve(new Response("{}", { status: 200 })));
    const html = renderToString(<App api={api} />);
    expect(html).toContain("Trace inspector");
    expect(html).toMatch(/<details[^>]*open=""[^>]*data-testid="details"/);
  });
});
