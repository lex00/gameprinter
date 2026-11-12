// The save state filmstrip under the screen (docs/show.md, "The
// filmstrip"): every save point of the run's free play sessions, grouped
// by session and branch with the newest at the end, each with its clip
// when it has one, and after them the clips the tool made, grouped by
// level (docs/show.md, "Clips made by the tool"). A strip of hundreds
// draws only the items in view (`visibleRange`). Pure, so the tests run
// it on made-up sessions.

import type { FreeSave, FreeSession } from "./freefold";
import type { Scenario } from "./scenariofold";
import type { ScreenScore } from "./screenfold";

/** A clip's place and the person's mark on it, as the session keeps them. */
export interface ClipMeta {
  save: number;
  branch: number | null;
  from: number;
  frames: number;
  cut: "auto" | "manual";
  ended: "save" | "load" | "cap" | "stop";
  mark: Mark;
}

export type Mark = "kept" | "skipped" | "starred";

/** A clip's score from the background queue: how close the rebuilt game stays from the original's state at its start. */
export interface ClipScore {
  /** Exact on every frame measured. */
  exact: boolean;
  /** Frames together, in thousandths. */
  permille: number;
  /** The frame of the clip where the two drift apart, if they do. */
  drift?: number | null;
  says?: string;
  /** Each track's first wrong frame, the bodies first and the camera apart. */
  tracks?: { track: string; frame: number; path: string; says: string }[];
  /** It didn't match before and does now. */
  fixed?: boolean;
  /** Why it couldn't be scored: the rebuilt game can't start there (a title screen). */
  error?: string | null;
  /** The two games' pictures compared: where they differ, by layer. */
  screen?: ScreenScore | null;
}

/** Where a clip came from: played by you, or made by the tool. */
export type ClipSource = "played" | "made";

/** One clip of a save point, or one the tool made at a start. */
export interface StripClip {
  id: string;
  /** The free play session, for a played clip; empty for one the tool made. */
  session: string;
  frames: number;
  meta?: ClipMeta;
  mark: Mark;
  score?: ClipScore;
  source: ClipSource;
  /** "World 1-1: walk right 3 s", for one the tool made. */
  name?: string;
  /** The clips it waits for, for one the tool made: the ladder's needs. */
  requires?: string[];
  /** One it waits for doesn't match yet, so the background work leaves it for now. */
  waits?: boolean;
}

/** A start the tool found (`GET /api/capture/{run}/generated`). */
export interface GenStart {
  id: string;
  recording: string;
  level: string;
  /** "World 1-2". */
  level_name: string;
  /** "World 1-2, before the first pit". */
  name: string;
  frame: number;
  says: string;
  /** Why the rebuilt game can't start here, when it can't. */
  cant?: string | null;
  clips: number;
  done: boolean;
}

/** A clip the tool made. */
export interface GenClip {
  id: string;
  name: string;
  start: string;
  level: string;
  level_name: string;
  tier: number;
  frames: number;
  requires: string[];
  mark: Mark;
  original?: { frames: number; moved: [number, number]; says: string };
}

/** The clips the tool made, as the server keeps them. */
export interface GenState {
  requested?: boolean;
  finished?: boolean;
  says?: string;
  error?: string | null;
  job?: { kind: "starts" | "clips"; k: number; n: number; recording?: string; start?: string; name?: string } | null;
  starts: GenStart[];
  clips: GenClip[];
}

export type StripItem =
  | { kind: "group"; key: string; session: string; branch: number; label: string; level?: string }
  | { kind: "save"; key: string; session: string; save: FreeSave; clip?: StripClip }
  | { kind: "made"; key: string; start: GenStart; clip?: StripClip };

/** "SESSION/K", how a clip's start names its save point. */
export function saveKey(session: string, k: number): string {
  return `${session}/${k}`;
}

/** The save point a clip starts at, from its start's name ("save point SESSION/K"). */
export function clipSave(c: Pick<Scenario, "start">): string | undefined {
  const m = /^save point (.+\/\d+)$/.exec(c.start?.input ?? "");
  return m?.[1];
}

/**
 * The strip's items: the sessions oldest first, in each its branches in
 * order with a small label before each, and in each branch its save points
 * in the order they were made. A branch starts at a load, and a save point
 * is made on the branch being played, so this is the order they were made.
 */
export function stripItems(sessions: FreeSession[], scores: Record<string, ClipScore> = {}): StripItem[] {
  const out: StripItem[] = [];
  const ordered = [...sessions].sort((a, b) => a.started_ms - b.started_ms || a.id.localeCompare(b.id));
  ordered.forEach((s, si) => {
    const clips = new Map<string, StripClip>();
    for (const c of s.clips ?? []) {
      const at = clipSave(c);
      if (!at) continue;
      const meta = s.clip_meta?.[c.id];
      // A save point recorded more than once keeps its newest clip.
      // A clip you played is starred unless you mark it otherwise.
      clips.set(at, { id: c.id, session: s.id, frames: c.frames, meta, mark: meta?.mark ?? "starred", score: scores[c.id], source: "played" });
    }
    const branches = [...new Set(s.saves.map((x) => x.branch))].sort((a, b) => a - b);
    for (const b of branches) {
      const br = s.branches.find((x) => x.id === b);
      const label = `${ordered.length > 1 ? `Session ${si + 1}, ` : ""}${br?.from_save ? `from save ${br.from_save}` : "from the start"}`;
      out.push({ kind: "group", key: `${s.id}#${b}`, session: s.id, branch: b, label });
      for (const sp of s.saves.filter((x) => x.branch === b).sort((x, y) => x.id - y.id)) {
        const key = saveKey(s.id, sp.id);
        out.push({ kind: "save", key, session: s.id, save: sp, clip: clips.get(key) });
      }
    }
  });
  return out;
}

/**
 * The clips the tool made as strip items: a group for each level in the
 * order the starts were found, then each start's clips in the ladder's
 * order. A start the rebuilt game can't begin at has one card saying so.
 */
export function madeItems(gen: GenState | undefined, scores: Record<string, ClipScore> = {}): StripItem[] {
  if (!gen) return [];
  const out: StripItem[] = [];
  let level: string | undefined;
  for (const st of gen.starts) {
    if (st.level !== level) {
      level = st.level;
      out.push({ kind: "group", key: `made#${st.level}`, session: "", branch: 0, label: st.level_name, level: st.level });
    }
    const clips = gen.clips.filter((c) => c.start === st.id);
    if (!clips.length) {
      out.push({ kind: "made", key: `made/${st.id}`, start: st });
      continue;
    }
    for (const c of clips) {
      out.push({
        kind: "made",
        key: `made/${c.id}`,
        start: st,
        clip: { id: c.id, session: "", frames: c.frames, mark: c.mark, score: scores[c.id], source: "made", name: c.name, requires: c.requires, waits: c.requires.some((r) => !scores[r]?.exact) },
      });
    }
  }
  return out;
}

export type Filter = "all" | "clip" | "not-matching" | "fixed" | "skipped";

/** Which clips the strip shows by where they came from. */
export type SourceFilter = "all" | ClipSource;

export const SOURCES: { id: SourceFilter; label: string }[] = [
  { id: "all", label: "everything" },
  { id: "played", label: "played by you" },
  { id: "made", label: "made by the tool" },
];

export const FILTERS: { id: Filter; label: string }[] = [
  { id: "all", label: "all" },
  { id: "clip", label: "has a clip" },
  { id: "not-matching", label: "not matching" },
  { id: "fixed", label: "fixed" },
  { id: "skipped", label: "skipped" },
];

/** What a clip's small mark says: its state in the background's work. "waits" is a clip the tool made whose parts don't match yet. */
export type ClipState = "waiting" | "waits" | "matches" | "not matching" | "fixed" | "skipped" | "no start";

export function clipState(c: StripClip): ClipState {
  if (c.mark === "skipped") return "skipped";
  // Not scored: waiting its turn, or (a clip the tool made) waiting for its parts to match.
  if (!c.score) return c.waits ? "waits" : "waiting";
  if (c.score.error) return "no start";
  if (c.score.exact) return c.score.fixed ? "fixed" : "matches";
  return "not matching";
}

/**
 * The items a filter keeps, by state and by where the clip came from; a
 * group stays when a card under it does. A start the rebuilt game can't
 * begin at shows only under "all" with the tool's clips.
 */
export function filterItems(items: StripItem[], f: Filter, source: SourceFilter = "all"): StripItem[] {
  if (f === "all" && source === "all") return items;
  const keep = (i: StripItem & { kind: "save" | "made" }) => {
    if (source !== "all" && (i.kind === "made" ? "made" : "played") !== source) return false;
    const c = i.clip;
    if (f === "all") return true;
    if (!c) return false;
    const st = clipState(c);
    if (f === "clip") return true;
    if (f === "skipped") return st === "skipped";
    if (f === "fixed") return st === "fixed";
    return st === "not matching";
  };
  const out: StripItem[] = [];
  let group: StripItem | undefined;
  for (const i of items) {
    if (i.kind === "group") group = i;
    else if (keep(i)) {
      if (group) out.push(group);
      group = undefined;
      out.push(i);
    }
  }
  return out;
}

/** The width of an item in the strip, in pixels: a save point's card, or a group's label. */
export const SAVE_W = 140;
export const GROUP_W = 28;

export function widthOf(i: StripItem): number {
  return i.kind === "group" ? GROUP_W : SAVE_W;
}

/** A card's key for the strip's steps: save points and the tool's clips. */
function isCard(i: StripItem): boolean {
  return i.kind !== "group";
}

/** Each item's left edge, and the strip's whole width last. */
export function offsets(items: StripItem[]): number[] {
  const out = [0];
  for (const i of items) out.push(out[out.length - 1] + widthOf(i));
  return out;
}

/**
 * The items to draw for a strip scrolled to `left` with `width` pixels in
 * view, and `overscan` more either side: [first, end).
 */
export function visibleRange(offs: number[], left: number, width: number, overscan = 4): [number, number] {
  const n = offs.length - 1;
  if (n <= 0) return [0, 0];
  // The first item whose right edge is past `left`.
  let lo = 0;
  let hi = n;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (offs[mid + 1] <= left) lo = mid + 1;
    else hi = mid;
  }
  let end = lo;
  while (end < n && offs[end] < left + width) end++;
  return [Math.max(0, lo - overscan), Math.min(n, end + overscan)];
}

/** The card next to `key` in the strip, `by` steps on (negative goes back); the last when `key` isn't in it. */
export function stepTo(items: StripItem[], key: string | undefined, by: number): string | undefined {
  const saves = items.filter(isCard);
  if (!saves.length) return undefined;
  const at = saves.findIndex((i) => i.key === key);
  if (at < 0) return saves[saves.length - 1].key;
  return saves[Math.max(0, Math.min(saves.length - 1, at + by))].key;
}

/** The scroll position that brings item `i` into a strip `width` wide, or the current one when it's already in view. */
export function scrollFor(offs: number[], i: number, left: number, width: number): number {
  if (i < 0 || i >= offs.length - 1) return left;
  if (offs[i] < left) return offs[i];
  if (offs[i + 1] > left + width) return offs[i + 1] - width;
  return left;
}

export interface Tally {
  /** Clips in the pool: played ones and the ones the tool made. */
  total: number;
  scored: number;
  matches: number;
  /** Not matching, but together on 90% of frames or more. */
  close: number;
  differs: number;
  noStart: number;
  /** Not scored yet: waiting their turn, or for the clips they need. */
  waiting: number;
  skipped: number;
}

/** How the clips stand, counted from the strip's items. */
export function tally(items: StripItem[]): Tally {
  const t: Tally = { total: 0, scored: 0, matches: 0, close: 0, differs: 0, noStart: 0, waiting: 0, skipped: 0 };
  for (const i of items) {
    if (i.kind === "group" || !i.clip) continue;
    t.total++;
    const st = clipState(i.clip);
    if (st === "skipped") t.skipped++;
    else if (st === "waiting" || st === "waits") t.waiting++;
    else if (st === "no start") (t.noStart++, t.scored++);
    else if (st === "matches" || st === "fixed") (t.matches++, t.scored++);
    else {
      t.scored++;
      if ((i.clip.score?.permille ?? 0) >= 900) t.close++;
      else t.differs++;
    }
  }
  return t;
}

/** The items of the active clips only, when the queue says which are active (a clip outside its pool isn't counted). */
export function activeItems(items: StripItem[], active: string[] | undefined): StripItem[] {
  if (!active) return items;
  const on = new Set(active);
  return items.filter((i) => i.kind === "group" || (i.clip !== undefined && on.has(i.clip.id)));
}

/** The tally in words: "120 of 535 clips scored: 18 match, 70 close (90% or more together), 32 differ; 75 can't start; 240 waiting." */
export function tallyWords(t: Tally): string {
  if (!t.total) return "No clips yet.";
  const parts = [`${t.matches} match`, `${t.close} close (90% or more together)`, `${t.differs} differ`];
  const rest = [t.noStart ? `${t.noStart} can't start` : "", t.waiting ? `${t.waiting} waiting` : "", t.skipped ? `${t.skipped} skipped` : ""].filter(Boolean);
  return `${t.scored} of ${t.total - t.skipped} clips scored: ${parts.join(", ")}${rest.length ? `; ${rest.join("; ")}` : ""}.`;
}
