// The control room's side of the page (docs/show.md, "The live cards"):
// one card for each job the background work runs now, with bars that
// move toward the goal while you watch, and the card of a job just done
// with its final score and the change since last time. The user asked for
// it on 2026-10-30: "a live render of stats of the ones in progress with
// bars showing moving toward the goal". Pure, so the tests run it on
// made-up states.

import type { BgDone, BgJob } from "./backfold";

/** One bar on a card: how far toward its goal, 0 to 1, and in words. No value is a line without a bar. */
export interface Bar {
  id: string;
  label: string;
  value?: number;
  says: string;
}

const frac = (k: number, n: number) => (n > 0 ? Math.min(1, Math.max(0, k / n)) : 0);
const pct = (v: number) => `${Math.round(v * 100)}%`;

/**
 * A job's bars. A score: frames together toward 100%, measures exact
 * toward all of them, the first wrong frame toward the clip's end (it
 * moves with the frames compared while none is wrong), and the frames
 * compared so far. A refine: the impact pass's trials, the fine pass's,
 * and the best split count so far against the start's, toward none.
 */
export function jobBars(j: BgJob): Bar[] {
  if (j.kind === "score") {
    const frames = j.frames ?? 0;
    const total = Math.max(j.total ?? 0, frames, 1);
    const measures = j.measures ?? 0;
    const exact = j.exact ?? 0;
    const fw = j.first_wrong ?? null;
    return [
      { id: "together", label: "frames together", value: frac(j.together ?? 0, frames), says: frames ? `${pct(frac(j.together ?? 0, frames))} so far` : "starting" },
      { id: "exact", label: "measures exact", value: frac(exact, measures), says: measures ? `${exact} of ${measures}` : "starting" },
      fw === null
        ? { id: "first-wrong", label: "first wrong frame", value: frac(frames, total), says: frames ? `none yet, ${frames} of ${total} frames` : "starting" }
        : { id: "first-wrong", label: "first wrong frame", value: frac(fw, total), says: `frame ${fw} of ${total}` },
      { id: "compared", label: "frames compared", value: frac(frames, total), says: `${frames} of ${total}` },
    ];
  }
  if (j.kind === "refine") {
    const base = j.base ?? null;
    const best = j.best ?? base;
    const bars: Bar[] = [
      { id: "impact", label: "impact pass", value: frac(j.impact_k ?? 0, j.impact_n ?? 0), says: j.impact_n ? `trial ${Math.min(j.impact_k ?? 0, j.impact_n)} of ${j.impact_n}` : "finding where they part" },
      { id: "fine", label: "fine pass", says: j.pass === "fine" ? `${j.fine_k ?? 0} trials on ${j.fine_knobs ?? 0} ${(j.fine_knobs ?? 0) === 1 ? "number" : "numbers"}` : "after the impact pass" },
    ];
    if (base !== null && best !== null)
      bars.push({ id: "best", label: "best so far", value: base > 0 ? frac(base - best, base) : 1, says: `${best} wrong, from ${base} at the start${j.kept ? `; ${j.kept} kept` : ""}` });
    return bars;
  }
  return [];
}

/** A card on the top row: a job running now, or one just done. */
export type LiveCard = { key: string; clip: string; name: string } & ({ state: "running"; job: BgJob } | { state: "done"; done: BgDone });

/**
 * The top row: every job running, by worker, then the jobs just done
 * (newest first, one card a clip, none for a clip running again) to fill
 * as many places as there are workers.
 */
export function liveCards(jobs: BgJob[] | undefined, done: BgDone[] | undefined, workers: number): LiveCard[] {
  const running = [...(jobs ?? [])].sort((a, b) => (a.slot ?? 0) - (b.slot ?? 0));
  const out: LiveCard[] = running.map((j, i) => ({ key: `run-${j.slot ?? i}-${j.kind}-${j.clip}`, clip: j.clip, name: j.name, state: "running", job: j }));
  const busy = new Set(running.map((j) => j.clip));
  const seen = new Set<string>();
  for (const d of [...(done ?? [])].reverse()) {
    if (out.length >= Math.max(workers, running.length)) break;
    if (busy.has(d.clip) || seen.has(d.clip)) continue;
    seen.add(d.clip);
    out.push({ key: `done-${d.seq}`, clip: d.clip, name: d.name, state: "done", done: d });
  }
  return out;
}

/** A done card's line: its final score and the change since last time. */
export function doneLine(d: BgDone): { says: string; change?: "up" | "down" | "same" } {
  if (d.error) return { says: "can't be scored: the rebuilt game can't start there" };
  const now = d.exact ? "matches exactly" : `${Math.round(d.after / 10)}% together`;
  if (d.before == null) return { says: `${now} (first score)` };
  const delta = Math.round(d.after / 10) - Math.round(d.before / 10);
  if (delta === 0) return { says: `${now}, as before`, change: "same" };
  return { says: `${now}, was ${Math.round(d.before / 10)}% (${delta > 0 ? "+" : ""}${delta})`, change: delta > 0 ? "up" : "down" };
}

/** The worker counts the control offers: 1 to the most. */
export function workerChoices(max: number | undefined): number[] {
  const m = Math.max(1, Math.floor(max ?? 1));
  return Array.from({ length: m }, (_, i) => i + 1);
}
