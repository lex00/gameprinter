// The game's own moments (docs/inspector.md, "Moments"): their lines in the
// model's words, the strip's ticks, the moments in a focus, finding one
// from a phrase, and "refine this" with the check on the others of its
// kind. The game and its numbers are made up.
import { describe, expect, it } from "vitest";
import { renderToString } from "react-dom/server";
import type { GameWords } from "../src/api";
import { MomentsStrip, RefineThisMoment } from "../src/Moments";
import { type Moment, type Moments, findMoment, inFocus, momentLine, momentName, momentsPhase, tick } from "../src/momentfold";
import { type AlsoCheck, type RefineState, alsoLine, momentRefinePhase, worseLine } from "../src/refinefold";

const g = {
  preview: true,
  vocab: {
    entities: [
      { name: "frog", role: "player", description: null },
      { name: "heron", role: "enemy", description: null },
    ],
    levels: [],
    counters: ["flies"],
    rules: {},
    actions: { jump: "a" },
  },
  mechanics: { mechanics: [], paths: [] },
} as unknown as GameWords;

const m = (kind: Moment["kind"], n: number, from: number, to: number, extra: Partial<Moment> = {}): Moment => ({
  kind,
  n,
  part: "entities.frog[0]",
  mechanic: kind === "jump" ? "platformer.jump" : null,
  from,
  to,
  seconds_from: from / 60,
  seconds_to: to / 60,
  peak: null,
  height: null,
  before: null,
  after: null,
  focus: { from: Math.max(1, from - 30), to: to + 30 },
  checked: true,
  start: from - 30,
  model: { from, to, offset: 0, peak: null, height: null, after: null },
  ...extra,
});

const jump1 = m("jump", 1, 218, 250, { height: 66, model: { from: 220, to: 248, offset: 2, peak: 236, height: 52, after: null } });
const jump2 = m("jump", 2, 400, 430, { height: 66, model: null });
const fly = m("counter", 1, 240, 240, { part: "counters.flies", before: "2", after: "3" });
const ms: Moments = {
  moments: [jump1, fly, m("landing", 1, 250, 250), jump2, m("life_lost", 1, 900, 900, { part: "counters.lives", model: null })],
  kinds: { jump: { original: 2, model: 1, mechanic: "platformer.jump" } },
  first_frame: 4,
  measured: 1000,
};

describe("moments", () => {
  it("says each moment in the model's words, with what the model does", () => {
    expect(momentLine(g, jump1)).toBe("Jump 1, 3.6 s: the original goes up 66 px; the model 52 px, 2 frames later");
    expect(momentLine(g, jump2)).toBe("Jump 2, 6.7 s: the original goes up 66 px; the model doesn't jump");
    expect(momentLine(g, fly)).toBe("Flies 1, 4.0 s: the flies go from 2 to 3; the model's flies do too");
    expect(momentLine(g, ms.moments[2])).toBe("Landing 1, 4.2 s: Frog lands; the model does too");
    expect(momentName(fly)).toBe("Flies 1");
  });
  it("places ticks over the slider's frames and lists the moments in a focus", () => {
    expect(tick(jump1, 200, 300)).toEqual({ left: 0.18, width: 0.32 });
    expect(inFocus(ms.moments, jump1.focus).map(momentName)).toEqual(["Jump 1", "Flies 1", "Landing 1"]);
    const html = renderToString(<MomentsStrip g={g} moments={ms.moments} lo={1} hi={1000} phase={momentsPhase(ms, false)} pick={() => undefined} />);
    expect(html.match(/data-testid="moment-tick"/g)?.length).toBe(5);
    expect(html).toContain("Jump 1, 3.6 s: the original goes up 66 px; the model 52 px, 2 frames later");
  });
  it("finds the moment a phrase names", () => {
    expect(findMoment(ms.moments, "refine the very first jump")).toBe(jump1);
    expect(findMoment(ms.moments, "the second jump")).toBe(jump2);
    expect(findMoment(ms.moments, "jump 2")).toBe(jump2);
    expect(findMoment(ms.moments, "the last jump")).toBe(jump2);
    expect(findMoment(ms.moments, "the first fly")).toBe(fly);
    expect(findMoment(ms.moments, "when I lose a life")?.kind).toBe("life_lost");
    expect(findMoment(ms.moments, "the third jump")).toBeUndefined();
  });
  it("says where finding the moments is", () => {
    expect(momentsPhase(undefined, true).state).toBe("loading");
    expect(momentsPhase(ms, false).says).toBe("5 moments found. 2 jumps, and the model does 1 of them.");
    expect(momentsPhase(undefined, false, "no mapping").state).toBe("failed");
  });
  it("refines one moment and reports the other moments of its kind, listing the ones made worse", () => {
    const also: AlsoCheck = {
      kind: "jump",
      others: 312,
      better: 280,
      worse: 1,
      same: 31,
      line: "",
      moment: null,
      worse_moments: [
        {
          n: 17,
          from: 4021,
          to: 4060,
          seconds: 66.9,
          original_height: 66,
          before: { apart: 3, found: true, offset: 0, height: 60, measures: [] },
          after: { apart: 12, found: true, offset: 0, height: 40, measures: [] },
        },
      ],
    };
    expect(alsoLine(also)).toBe("Also better on 280 of 312 other jumps, worse on 1.");
    expect(worseLine("jump", also.worse_moments[0])).toBe("Jump 17, 66.9 s: further from the original there (12 measure-frames apart, was 3); the model goes up 40 px, was 60 px, the original 66 px");
    const done = { recording: "r", state: "done", running: false, rounds: [{}], moment: { kind: "jump", n: 1 }, result: { also, stop: "", questions: [], first_split: {}, together: { before: 0, after: 0, measured: 0 } } } as unknown as RefineState;
    const phase = momentRefinePhase(done, "Jump 1", "platformer.jump");
    expect(phase.state).toBe("done");
    expect(phase.says).toBe("Done with jump 1: 1 number changed. Also better on 280 of 312 other jumps, worse on 1.");
    expect(momentRefinePhase(undefined, "Jump 1", "platformer.jump").says).toBe("Jump 1: refine tries jump's numbers in its frames only.");
    const html = renderToString(<RefineThisMoment moment={jump1} phase={phase} state={done} start={() => undefined} stop={() => undefined} seek={() => undefined} />);
    expect(html).toContain("refine this jump");
    expect(html).toContain("Jump 17, 66.9 s");
  });
});
