// The dashboard (docs/show.md, "Dashboard"): which clips get a card and in
// what order, each card's line, and the trend kept as scores change. The
// clips are made up.
import { describe, expect, it } from "vitest";
import { type Card, addScores, cardLine, inProgress } from "../src/Dashboard";
import type { StripClip } from "../src/stripfold";

const clip = (id: string, score?: StripClip["score"], extra: Partial<StripClip> = {}): Card =>
  ({ kind: "made", key: id, start: { id: "s" } as never, clip: { id, session: "", frames: 60, mark: "kept", source: "made", score, ...extra } }) as Card;

describe("the dashboard", () => {
  const a = clip("a", { exact: false, permille: 620, tracks: [{ track: "Mario", frame: 9, path: "entities.mario[0].vy", says: "" }] });
  const b = clip("b", { exact: true, permille: 1000 });
  const c = clip("c", { exact: false, permille: 400 });
  const w = clip("w");
  const n = clip("n", { exact: false, permille: 0, error: "can't start" });

  it("puts the clip worked on first, then recent changes, then the furthest from matching, and leaves out waiting and can't-start clips", () => {
    expect(inProgress([a, b, c, w, n], undefined, {}).map((x) => x.clip.id)).toEqual(["c", "a", "b"]);
    expect(inProgress([a, b, c, w, n], "w", {}).map((x) => x.clip.id)).toEqual(["w", "c", "a", "b"]);
    expect(inProgress([a, b, c], undefined, { b: 5, a: 9 }).map((x) => x.clip.id)).toEqual(["a", "b", "c"]);
  });

  it("says how together a clip is and where it first goes wrong", () => {
    expect(cardLine(a.clip)).toBe("62% together; Mario wrong from frame 9");
    expect(cardLine(b.clip)).toBe("matches exactly");
    expect(cardLine(w.clip)).toBe("waiting to be scored");
  });

  it("keeps each clip's scores as they change, at most 24", () => {
    const t1 = addScores({}, [a, b]);
    expect(t1).toEqual({ a: [620], b: [1000] });
    expect(addScores(t1, [a, b])).toBe(t1);
    const a2 = clip("a", { exact: false, permille: 810 });
    expect(addScores(t1, [a2]).a).toEqual([620, 810]);
  });
});
