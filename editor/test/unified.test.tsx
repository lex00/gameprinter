// The main screen (docs/show.md): the clip cutting rules on made-up
// frames, the filmstrip's grouping, filters, stepping and the window of
// cards it draws out of hundreds, and that the screen renders with
// nothing above the game.
import { describe, expect, it } from "vitest";
import { renderToString } from "react-dom/server";
import { Api } from "../src/api";
import { ClipDrawerFor, clipDoing, Unified } from "../src/Unified";
import { BackgroundClient, fixedToday, roundWords, statusSentence, type BgRound, type BgState } from "../src/backfold";
import { Filmstrip } from "../src/Filmstrip";
import { DEFAULT_SETTINGS, frameOfLine, capFrames, capSeconds, cutSays, emptyCut, hasPlay, keepClip, onFrame, onManualStart, onMark, onStop } from "../src/clipfold";
import { GROUP_W, SAVE_W, clipSave, clipState, filterItems, madeItems, offsets, scrollFor, stepTo, stripItems, visibleRange, type GenState, type StripItem } from "../src/stripfold";
import type { FreeSession } from "../src/freefold";
import { ScreenBoxes } from "../src/ScreenBoxes";
import { boxesAt, lockBoxes, type ScreenScore } from "../src/screenfold";

const s = DEFAULT_SETTINGS;

describe("clips cut from the play loop", () => {
  it("starts a clip at every save and load, and ends it at the next", () => {
    let c = emptyCut();
    const a = onMark(c, "save", 600, 600, 1, s);
    expect(a.ended).toBeUndefined();
    expect(a.state.open).toEqual({ from: 600, branch: 1, mark: 1, manual: false });
    c = a.state;
    // A save 5 seconds on ends it there and starts the next.
    const b = onMark(c, "save", 900, 900, 1, s);
    expect(b.ended).toMatchObject({ from: 600, to: 900, why: "save", mark: 1 });
    expect(b.state.open?.from).toBe(900);
    // A load ends it where play was, and starts one at the save point on the new branch.
    const l = onMark(b.state, "load", 1000, 600, 2, s);
    expect(l.ended).toMatchObject({ from: 900, to: 1000, why: "load", branch: 1 });
    expect(l.state.open).toEqual({ from: 600, branch: 2, mark: 3, manual: false });
  });

  it("ends a clip by itself at the cap", () => {
    const c = onMark(emptyCut(), "save", 100, 100, 1, s).state;
    expect(capFrames(s, false)).toBe(481);
    expect(onFrame(c, 100 + 480, s).ended).toBeUndefined();
    const e = onFrame(c, 100 + 481, s);
    expect(e.ended).toMatchObject({ from: 100, to: 581, why: "cap" });
    expect(e.state.open).toBeUndefined();
    // The cap is a setting.
    expect(onFrame(c, 100 + 121, { ...s, seconds: 2 }).ended?.why).toBe("cap");
    expect(capSeconds(0)).toBe(8);
    expect(capSeconds(90)).toBe(30);
  });

  it("cuts nothing with clips off, but start and stop by hand still work", () => {
    const off = { ...s, on: false };
    const a = onMark(emptyCut(), "save", 100, 100, 1, off);
    expect(a.state.open).toBeUndefined();
    const m = onManualStart(a.state, 200, 1);
    expect(m.state.open).toMatchObject({ from: 200, manual: true });
    // A clip started by hand runs to the longer cap.
    expect(onFrame(m.state, 200 + 600, off).ended).toBeUndefined();
    expect(onStop(m.state, 330).ended).toMatchObject({ from: 200, to: 330, why: "stop", manual: true });
    expect(cutSays(m.state, 260, off)).toMatch(/started by hand/);
    expect(cutSays(a.state, 0, off)).toMatch(/Clips: off/);
    expect(cutSays(emptyCut(), 0, s)).toMatch(/next save \(F5\) or load \(F8\) starts one/);
  });

  it("leaves out clips where nothing is played, unless the setting keeps them", () => {
    const idle = ["P ........", "- ........"];
    const startOnly = ["P ....T...", "P ........"];
    const hop = ["P ........", "P .......A", "P R......."];
    expect(keepClip([], s)).toEqual({ keep: false, why: "it has no frames" });
    expect(keepClip(idle, s)).toEqual({ keep: false, why: "nothing was pressed" });
    expect(keepClip(startOnly, s)).toEqual({ keep: false, why: "only Start or Select was pressed" });
    expect(keepClip(hop, s)).toEqual({ keep: true });
    expect(keepClip(idle, { ...s, dropIdle: false })).toEqual({ keep: true });
    expect(keepClip(idle, s, true)).toEqual({ keep: true });
    expect(hasPlay(["P! ......B."])).toBe(true);
    expect(frameOfLine("P! R......A")).toEqual({ pad: 0x81, reset: true, polled: true });
    expect(frameOfLine("- ........")).toEqual({ pad: 0, reset: false, polled: false });
  });
});

const save = (id: number, branch: number, frame: number) => ({ id, branch, frame, hash: "h", digest: "d", made_ms: id, picture: true, native: { hash: "h", digest: "d", matches: true } });
const session = (id: string, started: number, saves: ReturnType<typeof save>[], extra: Partial<FreeSession> = {}): FreeSession => ({
  schema: 0,
  id,
  run: "hop",
  runs: [],
  recording: null,
  frame: 0,
  started_ms: started,
  branches: [
    { id: 1, from_save: null, from_frame: 0, frames: 900 },
    { id: 2, from_save: 1, from_frame: 300, frames: 900 },
  ],
  saves,
  clips: [],
  ...extra,
});

describe("the filmstrip", () => {
  const clip = (id: string, at: string) => ({ id, name: id, tier: 0, mechanic: "", frames: 300, start: { input: `save point ${at}`, frame: 300 } }) as never;
  const one = session("free-1", 1, [save(1, 1, 300), save(2, 1, 600), save(3, 2, 500)], {
    clips: [clip("clip.1.1", "free-1/1"), clip("clip.1.2", "free-1/3")],
    clip_meta: { "clip.1.2": { save: 3, branch: 2, from: 500, frames: 300, cut: "auto", ended: "cap", mark: "skipped" } },
  });
  const two = session("free-2", 2, [save(1, 1, 120)]);

  it("groups save points by session and branch, the newest at the end", () => {
    const items = stripItems([two, one], { "clip.1.1": { exact: false, permille: 800 } });
    expect(items.map((i) => i.key)).toEqual(["free-1#1", "free-1/1", "free-1/2", "free-1#2", "free-1/3", "free-2#1", "free-2/1"]);
    expect(items[0]).toMatchObject({ kind: "group", label: "Session 1, from the start" });
    expect(items[3]).toMatchObject({ kind: "group", label: "Session 1, from save 1" });
    const card = items[1] as StripItem & { kind: "save" };
    expect(card.clip?.id).toBe("clip.1.1");
    expect(clipState(card.clip!)).toBe("not matching");
    expect(clipState({ ...card.clip!, score: { exact: false, permille: 0, error: "the game isn't under way" } })).toBe("no start");
    expect(clipSave({ start: { input: "save point free-1/12", frame: 1 } } as never)).toBe("free-1/12");
  });

  it("filters by clip and state, keeping a group's label with its cards", () => {
    const items = stripItems([one, two], { "clip.1.1": { exact: true, permille: 1000, fixed: true } });
    expect(filterItems(items, "clip").map((i) => i.key)).toEqual(["free-1#1", "free-1/1", "free-1#2", "free-1/3"]);
    expect(filterItems(items, "fixed").map((i) => i.key)).toEqual(["free-1#1", "free-1/1"]);
    expect(filterItems(items, "skipped").map((i) => i.key)).toEqual(["free-1#2", "free-1/3"]);
    expect(filterItems(items, "not-matching")).toEqual([]);
    expect(filterItems(items, "all")).toBe(items);
  });

  it("steps through the save points with the keys", () => {
    const items = stripItems([one, two]);
    expect(stepTo(items, "free-1/2", 1)).toBe("free-1/3");
    expect(stepTo(items, "free-1/2", -5)).toBe("free-1/1");
    expect(stepTo(items, "free-2/1", 10)).toBe("free-2/1");
    expect(stepTo(items, undefined, 1)).toBe("free-2/1");
  });

  it("draws only the cards in view out of hundreds", () => {
    const many = session(
      "free-9",
      9,
      Array.from({ length: 500 }, (_, i) => save(i + 1, 1, (i + 1) * 600)),
    );
    const items = stripItems([many]);
    const offs = offsets(items);
    expect(offs[offs.length - 1]).toBe(GROUP_W + 500 * SAVE_W);
    // 1000 pixels in view at the far end: 8 cards, and 4 either side at most.
    const [a, b] = visibleRange(offs, offs[offs.length - 1] - 1000, 1000);
    expect(b).toBe(501);
    expect(b - a).toBeLessThanOrEqual(8 + 4 + 1);
    const [c, d] = visibleRange(offs, 0, 1000);
    expect(c).toBe(0);
    expect(d - c).toBeLessThanOrEqual(1 + 8 + 4);
    expect(visibleRange([0], 0, 1000)).toEqual([0, 0]);
    // Bringing a card into view scrolls as little as it can.
    expect(scrollFor(offs, 10, 0, 1000)).toBe(offs[11] - 1000);
    expect(scrollFor(offs, 2, 0, 1000)).toBe(0);
    const html = renderToString(
      <Filmstrip items={items} total={500} picture={() => ""} choose={() => {}} load={() => {}} openClip={() => {}} filter="all" setFilter={() => {}} />,
    );
    // Before it's laid out it assumes a strip 1200 pixels wide, scrolled to the start.
    expect((html.match(/data-testid="strip-card"/g) ?? []).length).toBeLessThan(20);
    expect(html).toContain("500 save states");
  });
});

describe("clips made by the tool in the strip", () => {
  const start = (id: string, level: string, level_name: string, extra = {}) => ({ id, recording: "r", level, level_name, name: `${level_name}, start`, frame: 300, says: "", clips: 2, done: true, ...extra });
  const clip = (id: string, st: string, level: string, level_name: string, name: string, requires: string[] = []) => ({ id, name, start: st, level, level_name, tier: 1, frames: 100, requires, mark: "kept" as const });
  const gen: GenState = {
    starts: [start("w1_1.start", "w1_1", "World 1-1"), start("w1_1.pit", "w1_1", "World 1-1", { name: "World 1-1, before the first pit", cant: "enemies aren't carried over yet: the original has a goomba here", clips: 0 }), start("w1_2.start", "w1_2", "World 1-2")],
    clips: [
      clip("gen.w1_1.start.t1.nothing", "w1_1.start", "w1_1", "World 1-1", "World 1-1: nothing pressed for 60 frames"),
      clip("gen.w1_1.start.c.right.walk3s", "w1_1.start", "w1_1", "World 1-1", "World 1-1: walk right 3 s", ["gen.w1_1.start.t1.nothing"]),
      clip("gen.w1_2.start.t1.nothing", "w1_2.start", "w1_2", "World 1-2", "World 1-2: nothing pressed for 60 frames"),
    ],
  };
  const played = session("free-1", 1, [save(1, 1, 300)], { clips: [{ id: "clip.1.1", name: "c", tier: 0, mechanic: "", frames: 120, start: { input: "save point free-1/1", frame: 300 } } as never] });

  it("groups them by level after the save points, with a card for a start the rebuilt game can't begin at", () => {
    const items = [...stripItems([played]), ...madeItems(gen, { "gen.w1_1.start.t1.nothing": { exact: false, permille: 900 } })];
    expect(items.map((i) => i.key)).toEqual([
      "free-1#1",
      "free-1/1",
      "made#w1_1",
      "made/gen.w1_1.start.t1.nothing",
      "made/gen.w1_1.start.c.right.walk3s",
      "made/w1_1.pit",
      "made#w1_2",
      "made/gen.w1_2.start.t1.nothing",
    ]);
    expect(items[2]).toMatchObject({ kind: "group", label: "World 1-1" });
    const walk = items[4] as StripItem & { kind: "made" };
    expect(walk.clip).toMatchObject({ source: "made", name: "World 1-1: walk right 3 s", waits: true });
    // It waits for "nothing pressed" to match first.
    expect(clipState(walk.clip!)).toBe("waits");
    expect((items[1] as StripItem & { kind: "save" }).clip).toMatchObject({ source: "played", mark: "starred" });
  });

  it("filters by where a clip came from, keeping each level's label", () => {
    const items = [...stripItems([played]), ...madeItems(gen)];
    expect(filterItems(items, "all", "played").map((i) => i.key)).toEqual(["free-1#1", "free-1/1"]);
    const made = filterItems(items, "all", "made").map((i) => i.key);
    expect(made[0]).toBe("made#w1_1");
    expect(made).not.toContain("free-1/1");
    expect(made).toContain("made/w1_1.pit");
    // A state filter with a source: only clips, so the start that can't begin drops out.
    expect(filterItems(items, "clip", "made").map((i) => i.key)).toEqual([
      "made#w1_1",
      "made/gen.w1_1.start.t1.nothing",
      "made/gen.w1_1.start.c.right.walk3s",
      "made#w1_2",
      "made/gen.w1_2.start.t1.nothing",
    ]);
    // The keys step through the tool's cards too.
    expect(stepTo(items, "free-1/1", 1)).toBe("made/gen.w1_1.start.t1.nothing");
  });

  it("draws the tool's cards with their names, the source filters and why a start can't begin", () => {
    const items = [...stripItems([played]), ...madeItems(gen)];
    const html = renderToString(
      <Filmstrip items={items} total={1} made={3} picture={() => ""} madePicture={(i) => `/p/${i.start.id}`} choose={() => {}} load={() => {}} openClip={() => {}} filter="all" setFilter={() => {}} source="all" setSource={() => {}} />,
    ).replace(/<!-- -->/g, "");
    expect(html).toContain('data-testid="strip-source-played"');
    expect(html).toContain(">played by you</button>");
    expect(html).toContain(">made by the tool</button>");
    expect(html).toContain("walk right 3 s");
    expect(html).toContain('src="/p/w1_1.start"');
    expect(html).toContain("enemies aren&#x27;t carried over yet");
    expect(html).toContain("3 of 3 clips made by the tool");
  });

  it("says in the background line what the making does", () => {
    const st: BgState = { enabled: false, scores: {}, questions: [], generating: { active: true, says: "Generating: World 1-3, 12 of 40." } };
    expect(statusSentence(st, [], 0)).toBe("Generating: World 1-3, 12 of 40. Background: off; turn it on to score your clips and fix numbers while you play");
    expect(statusSentence({ ...st, generating: { active: false, says: "Made 3 clips." } }, [], 0)).not.toContain("Made");
  });
});

describe("the main screen", () => {
  it("puts a compact header first, then the game, the view row and the thumbnails; the rest is behind more and record", () => {
    const html = renderToString(<Unified api={new Api("")} run="hop" />).replace(/<!-- -->/g, "");
    const at = (t: string) => html.indexOf(t);
    expect(at('data-testid="controls"')).toBeLessThan(at('data-testid="screen-original"'));
    expect(at('data-testid="screen-original"')).toBeLessThan(at('data-testid="view-row"'));
    expect(at('data-testid="view-row"')).toBeLessThan(at('data-testid="thumbs"'));
    expect(html).toContain('data-testid="fidelity-gauge"');
    expect(html).toContain('data-testid="ctl-workers"');
    expect(html).toContain('data-testid="ctl-agents"');
    // The rebuilt game is hidden until asked for.
    expect(html).not.toContain('data-testid="screen-model"');
    // The recorder is one button; its controls and the strip are tucked away.
    expect(html).toContain(">record</button>");
    expect(html).not.toContain("save (F5)");
    expect(html).not.toContain('data-testid="filmstrip"');
    expect(html).not.toContain('data-testid="ctl-clip-on"');
    // No long text panels by default.
    expect(html).not.toContain('data-testid="progress"');
    expect(html).not.toContain('data-testid="bg-line"');
    expect(html).not.toContain('data-testid="live-cards"');
  });

  it("opens a clip with a status chip, where it parts, its question big, and keep, skip and star", () => {
    const item = stripItems([session("free-1", 1, [save(1, 1, 2470)], { clips: [{ id: "clip.1.1", name: "c", tier: 0, mechanic: "", frames: 120, start: { input: "save point free-1/1", frame: 2470 } } as never] })])[1] as StripItem & { kind: "save" };
    const q = { id: "q1", clip: "clip.1.1", clip_name: "c", at: 60, ask: "Watch the clip: how does the rebuilt game look where the two part?", choices: ["too high", "too low", "looks right"], detail: "the long detail", model: "m" };
    const w = { clip: item.clip!, item, question: q };
    const bg: BgState = { enabled: true, scores: {}, questions: [q] };
    const html = renderToString(
      <ClipDrawerFor w={w} ctx={{ bg, now: 0 }} score={{ exact: false, permille: 750, drift: 60, drift_path: "entities.frog[0].y" }} at={30} mark={() => {}} fix={() => {}} working={false} playOn={() => {}} answer={() => {}} close={() => {}} />,
    ).replace(/<!-- -->/g, "");
    expect(html).toContain("Played by you, 2.0 s");
    // A clip you recorded: its question needs you, big, with the detail behind a disclosure.
    expect(html).toContain('data-testid="clip-status">needs you<');
    expect(html).toContain('class="q big"');
    expect(html).toMatch(/<button class="primary">too high<\/button>/);
    expect(html).toMatch(/<details><summary>details<\/summary><p class="hint">the long detail<\/p>/);
    expect(html).toContain('data-testid="clip-drift"');
    expect(html).toContain("from frame 60");
    for (const b of ["fix this clip", "keep", "skip", "star", "play on from here"]) expect(html).toContain(`>${b}</button>`);
    // A clip you played is starred unless you mark it otherwise.
    expect(html).toMatch(/class="on"[^>]*data-testid="clip-starred"/);
    // The long words are in the details.
    expect(html.indexOf("They drift apart at 1.0 s")).toBeGreaterThan(html.indexOf("<summary>details</summary>"));
  });

  it("shows the loop's generic question on a clip the tool made only behind a disclosure, not as needs you", () => {
    const made = { kind: "made" as const, key: "made/gen.a", start: { id: "s", name: "World 1-1, start" } as never, clip: { id: "gen.a", session: "", frames: 120, mark: "kept" as const, source: "made" as const, name: "World 1-1: walk" } };
    const q = { id: "q9", clip: "gen.a", clip_name: "a", at: 6, ask: "Watch the clip: how does the rebuilt game look where the two part?", choices: ["too fast", "too slow", "looks right"], detail: "", model: "m" };
    const html = renderToString(
      <ClipDrawerFor w={{ clip: made.clip, item: made }} ctx={{ bg: { enabled: true, scores: {}, questions: [q] }, now: 0 }} score={{ exact: false, permille: 950, drift: 6 }} at={0} mark={() => {}} fix={() => {}} working={false} answer={() => {}} close={() => {}} />,
    ).replace(/<!-- -->/g, "");
    expect(html).not.toContain('data-testid="clip-question"');
    expect(html).toContain('data-testid="clip-status">close<');
    expect(html).toContain("<summary>The loop&#x27;s question</summary>");
  });
});

describe("where the pictures differ", () => {
  const screen: ScreenScore = {
    sprites: { name: "screen.sprites", owner: "presentation", same: 2, total: 4, first: 1, boxes: [{ x: 10, y: 12, w: 5, h: 4, what: ["frog"] }] },
    background: { name: "screen.background", owner: "core.camera", same: 3, total: 4, first: 40, shift: [-3, 0] },
    says: [
      "Characters: 50% of frames the same; different from frame 1: 1 place (frog).",
      "Background: 75% of frames the same; shifted from frame 40 (the camera): 3 px further left in the rebuilt game.",
    ],
    spans: [
      { from: 1, to: 3, layer: "sprites", boxes: [[10, 12, 5, 4]] },
      { from: 40, to: 40, layer: "background", boxes: [[0, 32, 256, 200]] },
    ],
  };
  it("finds each frame's boxes, both layers", () => {
    expect(boxesAt(screen, 0)).toEqual([]);
    expect(boxesAt(screen, 2)).toEqual([{ layer: "sprites", x: 10, y: 12, w: 5, h: 4 }]);
    expect(boxesAt(screen, 40)).toEqual([{ layer: "background", x: 0, y: 32, w: 256, h: 200 }]);
    expect(boxesAt(undefined, 2)).toEqual([]);
    const lock = { visible: [0, 8, 256, 224] as [number, number, number, number], sprites: { frames: [{ boxes: [] }, { boxes: [{ x: 1, y: 2, w: 3, h: 4 }] }] }, background: { frames: [{ boxes: [] }, { boxes: [] }] } };
    expect(lockBoxes(lock, 1)).toEqual([{ layer: "sprites", x: 1, y: 2, w: 3, h: 4 }]);
    expect(lockBoxes(lock, 5)).toEqual([]);
  });

  it("says each layer in the clip's details, with the toggle", () => {
    const item = stripItems([session("free-1", 1, [save(1, 1, 2470)], { clips: [{ id: "clip.1.1", name: "c", tier: 0, mechanic: "", frames: 120, start: { input: "save point free-1/1", frame: 2470 } } as never] })])[1] as StripItem & { kind: "save" };
    const w = { clip: item.clip! , item };
    const html = renderToString(
      <ClipDrawerFor w={w} ctx={{ now: 0 }} score={{ exact: false, permille: 750, drift: 60, screen }} at={2} mark={() => {}} fix={() => {}} working={false} playOn={() => {}} answer={() => {}} close={() => {}} />,
    ).replace(/<!-- -->/g, "");
    expect(html).toContain("Characters: 50% of frames the same; different from frame 1: 1 place (frog).");
    expect(html).toContain("Background: 75% of frames the same; shifted from frame 40 (the camera)");
    expect(html).toContain("show where the pictures differ");
    expect(html).toMatch(/type="checkbox" checked=""[^>]*data-testid="clip-boxes"|data-testid="clip-boxes"[^>]*checked/);
    const svg = renderToString(<ScreenBoxes boxes={boxesAt(screen, 2)} />);
    expect(svg).toContain('data-layer="sprites"');
    expect(svg).toContain('viewBox="0 0 256 240"');
  });
});

describe("the background line", () => {
  const now = new Date(2026, 9, 29, 15, 0).getTime();
  const base: BgState = { enabled: true, working: true, scores: {}, questions: [] };
  const hist: BgRound[] = [
    { round: 1, when_ms: new Date(2026, 9, 28, 20, 0).getTime(), changes: [], fixed: 4, says: "" },
    { round: 2, when_ms: now - 1000, changes: [{ param: "entities.frog.jump.gravity", old: "0.1875", new: "0.25" }], fixed: 3, worse: [], says: "" },
  ];
  it("says what the queue does now, what it fixed today and the questions", () => {
    const q = { id: "q1", clip: "c", clip_name: "c", ask: "", choices: [], detail: "", model: "m" };
    expect(statusSentence({ ...base, job: { kind: "score", clip: "c", name: "c", k: 14, n: 52 }, questions: [q] }, hist, now)).toBe(
      "Background: scoring clip 14 of 52; 3 fixed today; 1 question for you",
    );
    expect(statusSentence({ ...base, working: false, enabled: false }, [], now)).toMatch(/^Background: off; turn it on/);
    expect(statusSentence({ ...base, working: false, enabled: false }, hist, now)).toBe("Background: paused; 3 fixed today");
    expect(statusSentence({ ...base, working: false, scores: { a: { model: "m", exact: true, permille: 1000, score: 0 }, b: { model: "m", exact: false, permille: 900, score: 3 } } }, [], now)).toBe(
      "Background: waiting; 1 of 2 clips differ",
    );
    expect(statusSentence(undefined, [], now)).toBe("Background: checking.");
    expect(fixedToday(hist, now)).toBe(3);
  });
  it("says each round in the game's words", () => {
    expect(roundWords(undefined, hist[1])).toMatch(/^Round 2: .* 0.1875 -> 0.25, 3 clips fixed, none worse\.$/);
    expect(roundWords(undefined, { round: 3, when_ms: 0, stuck: true, clip_name: "Clip 2", says: "" })).toBe("Round 3: no numbers make Clip 2 match; a question for you.");
    expect(roundWords(undefined, { when_ms: 0, answer: "q1", choice: "too high", says: "You said too high." })).toBe("You said too high.");
  });
  it("sends what the server expects", async () => {
    const calls: [string, string | undefined][] = [];
    const api = new Api("", (input, init) => {
      calls.push([String(input), init?.method]);
      return Promise.resolve(new Response("{}", { status: 200, headers: { "Content-Type": "application/json" } }));
    });
    const c = new BackgroundClient(api);
    await c.state("hop");
    await c.start("hop");
    await c.stop("hop");
    await c.history("hop");
    await c.answer("hop", "q1", "too high");
    expect(calls.map((x) => x[0])).toEqual([
      "/api/capture/hop/background",
      "/api/capture/hop/background/start",
      "/api/capture/hop/background/stop",
      "/api/capture/hop/background/history",
      "/api/capture/hop/background/answer",
    ]);
  });
});

describe("what the background work is doing for a clip", () => {
  const base: BgState = { enabled: true, working: true, scores: {}, questions: [], focus: "c1" };
  it("says it failed, works on it now with what it tries, waits, or is paused", () => {
    expect(clipDoing({ ...base, error: "the run has no recording" }, "c1")).toEqual({ says: "The background work stopped: the run has no recording.", failed: true, busy: false });
    expect(clipDoing({ ...base, job: { kind: "refine", clip: "c1", name: "c", round: 2, trying: { param: "entities.mario.jump.gravity_held", value: "0.15625" } } }, "c1")?.says).toBe("Working on this clip now (round 2). Trying jump gravity_held = 0.15625.");
    expect(clipDoing({ ...base, job: { kind: "score", clip: "c2", name: "d" }, says: "Scoring clip 2 of 5." }, "c1")?.says).toBe("Waiting: Scoring clip 2 of 5.");
    expect(clipDoing({ ...base, enabled: false }, "c1")?.says).toMatch(/paused/);
    expect(clipDoing({ ...base, enabled: false, focus: null }, "c1")).toBeUndefined();
  });
});
