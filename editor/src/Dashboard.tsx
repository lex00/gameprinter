// Each clip's score as this page saw it change (docs/show.md, "The main
// screen"): the trend kept in the browser, when each clip last changed
// (the thumbnails' "changed recently" and "got better"), a small
// sparkline of it, and the older dashboard order and card line. The user
// asked for moving stats on 2026-10-30; the thumbnails (Calm.tsx) show
// them now.

import { useEffect, useRef, useState } from "react";
import { type StripClip, type StripItem, clipState } from "./stripfold";

export type Card = StripItem & { kind: "save" | "made"; clip: StripClip };

/** A clip's scores as this page saw them change, oldest first, by clip. */
export type Trends = Record<string, number[]>;

const TRENDS_KEY = "gp.dashboard.trends";
const KEEP = 24;

export function readTrends(): Trends {
  try {
    return JSON.parse(localStorage.getItem(TRENDS_KEY) ?? "{}") as Trends;
  } catch {
    return {};
  }
}

function saveTrends(t: Trends): void {
  try {
    localStorage.setItem(TRENDS_KEY, JSON.stringify(t));
  } catch {
    // Not kept; the trend starts again next time.
  }
}

/** The trends with each card's current score added when it changed. */
export function addScores(t: Trends, cards: Card[]): Trends {
  let out = t;
  for (const c of cards) {
    const s = c.clip.score;
    if (!s || s.error) continue;
    const v = s.exact ? 1000 : s.permille;
    const was = t[c.clip.id] ?? [];
    if (was[was.length - 1] === v) continue;
    if (out === t) out = { ...t };
    out[c.clip.id] = [...was, v].slice(-KEEP);
  }
  return out;
}

/**
 * The cards in progress, in order: the one worked on now, then those whose
 * score changed most recently on this page, then those that don't match
 * yet by score (furthest first), then those that match. Clips waiting,
 * skipped or that can't start are left out; the progress panel counts them.
 */
export function inProgress(items: StripItem[], working: string | undefined, changed: Record<string, number>): Card[] {
  const cards = items.filter((i): i is Card => i.kind !== "group" && !!i.clip);
  const shown = cards.filter((c) => {
    const st = clipState(c.clip);
    return c.clip.id === working || st === "not matching" || st === "matches" || st === "fixed";
  });
  const rank = (c: Card) => {
    if (c.clip.id === working) return [0, 0];
    const at = changed[c.clip.id];
    if (at !== undefined) return [1, -at];
    const st = clipState(c.clip);
    if (st === "not matching") return [2, c.clip.score?.permille ?? 0];
    return [3, 0];
  };
  return shown.sort((a, b) => {
    const [ra, sa] = rank(a);
    const [rb, sb] = rank(b);
    return ra - rb || sa - sb || a.clip.id.localeCompare(b.clip.id);
  });
}

/** A card's main line: "62% together; Mario wrong from frame 9", or "matches exactly". */
export function cardLine(c: StripClip): string {
  const s = c.score;
  if (!s) return "waiting to be scored";
  if (s.exact) return s.fixed ? "matches exactly (fixed)" : "matches exactly";
  const first = s.tracks?.[0];
  const where = first ? `; ${first.track} wrong from frame ${first.frame}` : s.drift != null ? `; drifts at frame ${s.drift}` : "";
  return `${Math.round(s.permille / 10)}% together${where}`;
}

export function Sparkline({ values }: { values: number[] }) {
  if (values.length < 2) return null;
  const w = 80;
  const h = 18;
  const lo = Math.min(...values, 0);
  const hi = 1000;
  const pts = values.map((v, i) => `${((i / (values.length - 1)) * w).toFixed(1)},${(h - ((v - lo) / Math.max(1, hi - lo)) * h).toFixed(1)}`).join(" ");
  return (
    <svg className="dash-spark" viewBox={`0 0 ${w} ${h}`} width={w} height={h} aria-label="its score over time" role="img">
      <polyline points={pts} />
    </svg>
  );
}

/** The trends of the clips shown, and when each last changed on this page (ms). */
export function useTrends(cards: Card[]): { trends: Trends; changed: Record<string, number> } {
  const [trends, setTrends] = useState<Trends>(readTrends);
  const changed = useRef<Record<string, number>>({});
  useEffect(() => {
    const next = addScores(trends, cards);
    if (next === trends) return;
    const now = Date.now();
    for (const [id, vs] of Object.entries(next)) if (vs !== trends[id] && trends[id]) changed.current[id] = now;
    setTrends(next);
    saveTrends(next);
  }, [cards]); // eslint-disable-line react-hooks/exhaustive-deps
  return { trends, changed: changed.current };
}
