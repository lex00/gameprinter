// Credit for the person's notes and fixes waiting to merge (docs/show.md,
// "Telling the agent"): the chip on a tell row, the star and fix-ready
// badges, the header counters and the needs-you exclusion.
import { describe, expect, it } from "vitest";
import { renderToString } from "react-dom/server";
import { type WorkState, type WorkTell, headerCounts } from "../src/agentfold";
import { type Ctx, clipBadges, clipStatus, needsYouClips, thumbOrder } from "../src/calmfold";
import { BadgeChip, ClipDetail } from "../src/Calm";
import type { BgState } from "../src/backfold";

const NOW = 1_000_000_000;
const tell = (id: number, clip: string, credit: WorkTell["credit"]): WorkTell => ({ id, clip, text: "the hop is too low", by: "user", when_ms: id, open: false, replies: [], credit });
const fixed = (when: number): WorkTell["credit"] => ({
  status: "fixed",
  words: "fixed thanks to your note",
  change: "97% -> exact",
  attempt: 117,
  when_ms: when,
  before: { permille: 970, score: 4, exact: false },
  after: { permille: 1000, score: 0, exact: true },
});
const work = (extra: Partial<WorkState> = {}): WorkState => ({ run: "r", working: [], attempts: [], entries: [], ...extra });
const ready = { count: 1, fixes: [{ clip: "c1", attempt: 117, title: "gravity", exact: true, says: "A fix is ready: attempt 117 makes this clip exact. It's waiting to be merged." }] };
// An open question on a clip the person recorded: needs them.
const bgAsk = { questions: [{ id: "q1", clip: "c1", clip_name: "c1", ask: "Watch it", choices: ["looks right"], detail: "", answer: null }] } as unknown as BgState;
const clip = { id: "c1", score: { exact: false, permille: 900, score: 5 } } as never;

const drawer = (extra: object = {}) =>
  renderToString(
    <ClipDetail name="Hop" sub="x" status="differs" frames={120} at={40} attempts={[]} rounds={[]} watch={() => {}} keep={() => {}} needsYou={[]} other={[]} asks={[]} answer={() => {}} mark="kept" setMark={() => {}} close={() => {}} tell={async () => {}} {...extra} />,
  );

describe("a note's credit in the drawer", () => {
  it("shows a green chip and before -> after on a resolved tell", () => {
    const html = drawer({ tells: [tell(1, "c1", fixed(NOW))] });
    expect(html).toContain('data-testid="tell-credit"');
    expect(html).toContain("chip resolved");
    expect(html).toContain("fixed thanks to your note");
    expect(html).toContain("97% -&gt; exact");
    expect(html).toContain("attempt 117");
  });
  it("says improved when no attempt is linked, and nothing on an open one", () => {
    const html = drawer({ tells: [tell(1, "c1", { status: "improved", words: "improved since your note", change: "62% -> 81%" })] });
    expect(html).toContain("improved since your note");
    expect(drawer({ tells: [tell(1, "c1", { status: "open", words: "still open" })] })).not.toContain("tell-credit");
  });
  it("says fixed once merged while the fix waits, with a line at the top", () => {
    const html = drawer({ tells: [tell(1, "c1", { status: "pending", words: "fixed once merged", attempt: 117 })], fixReady: ready.fixes[0] });
    expect(html).toContain("fixed once merged");
    expect(html).toContain("A fix is ready: attempt 117 makes this clip exact. It&#x27;s waiting to be merged.");
  });
  it("hides Needs you while a fix is ready", () => {
    const q = { id: "q1", clip: "c1", clip_name: "c1", ask: "Watch it", choices: ["looks right"], detail: "", answer: null } as never;
    expect(drawer({ needsYou: [q] })).toContain("clip-question");
    expect(drawer({ needsYou: [q], fixReady: ready.fixes[0] })).not.toContain("clip-question");
  });
});

describe("badges and the header", () => {
  it("shows the green star for a note just resolved, and drops it later", () => {
    const w = work({ tells: [tell(1, "c1", fixed(NOW - 60_000))] });
    expect(clipBadges({ work: w, now: NOW }, clip).map((b) => b.kind)).toContain("resolved");
    expect(clipBadges({ work: w, now: NOW + 10 * 60_000 }, clip).map((b) => b.kind)).not.toContain("resolved");
    expect(renderToString(<BadgeChip b={{ kind: "resolved", label: "your note" }} />)).toContain('data-testid="badge-resolved"');
  });
  it("puts fix ready first, over being fixed, and never needs you", () => {
    const w = work({ fix_ready: ready, working: [{ by: "clip agent", clip: "c1", when_ms: 1, says: "x" }] });
    const ctx: Ctx = { bg: bgAsk, work: w, now: NOW };
    expect(clipBadges(ctx, clip).map((b) => b.kind)).toEqual(["fix-ready"]);
    expect(renderToString(<BadgeChip b={{ kind: "fix-ready", label: "fix ready" }} />)).toContain("fix ready");
    expect(clipStatus(ctx, clip)).toBe("fix ready");
    expect(needsYouClips(bgAsk, w).has("c1")).toBe(false);
    // Without the fix it does need the person.
    expect(needsYouClips(bgAsk, work()).has("c1")).toBe(true);
    expect(clipBadges({ bg: bgAsk, work: work(), now: NOW }, clip).map((b) => b.kind)).toContain("needs-you");
  });
  it("lists a fix-ready clip first", () => {
    const w = work({ fix_ready: ready });
    const cards = [{ clip: { id: "a", score: undefined } }, { clip: { id: "c1", score: undefined } }] as never[];
    const out = thumbOrder({ bg: { questions: [{ id: "q", clip: "a", clip_name: "a", ask: "?", choices: [], detail: "", answer: null, kind: "conflict" }] } as unknown as BgState, work: w, now: NOW }, cards);
    expect((out[0] as { clip: { id: string } }).clip.id).toBe("c1");
  });
  it("counts notes fixed and fixes ready, quietly", () => {
    expect(headerCounts(undefined)).toEqual({});
    expect(headerCounts(work({ tell_tally: { resolved: 4, fixed: 4, improved: 0, exact: 2, frames: 30, open: 1 } })).notes).toBe("your notes: 4 fixed");
    expect(headerCounts(work({ tell_tally: { resolved: 5, fixed: 4, improved: 1, exact: 2, frames: 30, open: 0 } })).notes).toBe("your notes: 4 fixed, 1 improved");
    expect(headerCounts(work({ fix_ready: { count: 2, fixes: [] } })).ready).toBe("2 fixes ready to merge");
    expect(headerCounts(work({ fix_ready: { count: 1, fixes: [] } })).ready).toBe("1 fix ready to merge");
  });
});
