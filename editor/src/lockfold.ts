// The lockstep panel's words and numbers (docs/inspector.md, "The game
// view"): what `GET /api/capture/{run}/lockstep` gives (the original and
// the model on one input, every mapped path's distance each frame, the
// splits and the mechanics they belong to), named in the model's words.
// Nothing here names a game.

import type { LockScreen } from "./screenfold";
import type { GameWords } from "./api";
import { count, list, thing, words } from "./phrase";
import { NES_FPS } from "./playfold";

export type LockMode = "resync" | "free";

export interface LockPath {
  path: string;
  level: string;
  tolerance: number;
  within: number;
  compared: number;
  mean: number;
  max: number;
  first_split: { frame: number; original_frame: number } | null;
  owners: string[];
  distance: number[];
  /** The distance past which the two have drifted apart (the server's; else from the units). */
  drift?: number;
}

export interface LockSplit {
  at: number;
  frame: number;
  original_frame: number;
  paths: { path: string; original: string; model: string; distance: number }[];
}

export interface LockNote {
  path: string;
  how: "loaded" | "approximated" | "missing";
  why: string;
}

export interface Lockstep {
  mode: LockMode;
  recording?: string;
  measured: number;
  /** Each measured frame: [model frame, original frame]. */
  frames: [number, number][];
  paths: LockPath[];
  splits: LockSplit[];
  mechanics: { mechanic: string; splits: number; paths: string[]; first: { frame: number; original_frame: number } }[];
  start: { frame: number; original_frame: number; notes: LockNote[] } | null;
  resync_notes: LockNote[];
  /** The two games' pictures compared on each measured frame, when the model's pictures could be drawn. */
  screen?: LockScreen;
}

const FIELD: Record<string, string> = {
  x: "position across",
  y: "height",
  stage_x: "position across",
  stage_y: "height",
  vx: "speed across",
  vy: "speed up and down",
  on_ground: "standing",
  facing: "facing",
  alive: "being alive",
  waiting: "having appeared",
  state: "state",
  time: "time in its state",
  hp: "damage",
};

const ORDINAL = ["", "second ", "third ", "fourth ", "fifth "];

/** A measure as a person says it: "Mario's speed up and down", "the score", "the view's position across". */
export function measureName(g: GameWords | undefined, path: string): string {
  const m = /^entities\.([a-z0-9_]+)\[(\d+)\]\.([a-z_]+)$/.exec(path);
  if (m) {
    const n = Number(m[2]);
    const who = n === 0 ? thing(g, m[1]) : `the ${ORDINAL[n] ?? `${n + 1}th `}${words(m[1])}`;
    return `${who}'s ${FIELD[m[3]] ?? words(m[3])}`;
  }
  const parts = path.split(".");
  if (parts[0] === "counters") return `the ${words(parts[1] ?? "")}`;
  if (parts[0] === "cameras") return `the view's ${FIELD[parts[2] ?? ""] ?? words(parts[2] ?? "")}`;
  if (parts[0] === "rules") {
    const rule = parts[1] === "game" ? "what the game is doing" : `the ${words(parts[1] ?? "")} rule`;
    return parts[2] === "time" ? `how long ${rule === "what the game is doing" ? "the game has been doing it" : `${rule} has been in its state`}` : rule;
  }
  if (path === "level") return "the level";
  return words(path);
}

/** The unit a measure's distance is in. */
export function measureUnit(path: string): string {
  const f = path.split(".").pop() ?? "";
  if (["x", "y", "stage_x", "stage_y"].includes(f)) return "px";
  if (["vx", "vy"].includes(f)) return "px a frame";
  if (path.startsWith("counters.")) return "";
  return "apart (1) or together (0)";
}

/** A mechanic as a person says it: its sentence from the catalog, or its name. */
export function mechanicName(g: GameWords | undefined, name: string): string {
  const m = g?.mechanics.mechanics.find((x) => x.name === name);
  const short = words(name.split(".").pop() ?? name);
  return m?.about ? `${short} (${m.about.charAt(0).toLowerCase()}${m.about.slice(1)})` : short;
}

/** Seconds of the original's run, as the view counts them. */
export function seconds(frame: number): string {
  return `${(frame / NES_FPS).toFixed(1)} seconds`;
}

/**
 * The one-line summary: what agrees on every frame, and what splits first
 * and when. "The two agree on the score and the coins on every frame;
 * Mario's speed up and down splits first, at 5.8 seconds (jump)."
 */
export function lockSummary(g: GameWords | undefined, ls: Lockstep): string {
  const together = ls.paths.filter((p) => !p.first_split).map((p) => measureName(g, p.path));
  const apart = ls.paths.filter((p) => p.first_split).sort((a, b) => a.first_split!.original_frame - b.first_split!.original_frame);
  if (!ls.measured) return "The two games were never in step: the original didn't reach the place the model starts.";
  if (!apart.length) return `The two agree on everything measured (${list(together)}) on all ${ls.measured} frames.`;
  const first = apart[0];
  const owner = first.owners[0] ? ` (${words(first.owners[0].split(".").pop() ?? "")})` : "";
  const head = together.length ? `The two agree on ${list(together)} on every frame; ` : "No measure agrees on every frame; ";
  const split = `${measureName(g, first.path)} splits first, at ${seconds(first.first_split!.original_frame)}${owner}`;
  const tail =
    ls.mode === "resync"
      ? `. Put back at every split, they part ${count(ls.splits.length, "time")}.`
      : `, and from there the two drift apart.`;
  return cap(head + split + tail);
}

const cap = (s: string) => (s ? s[0].toUpperCase() + s.slice(1) : s);

/** The original frames the chart's points sit at, and each one's distance. */
export function series(ls: Lockstep, i: number): { frame: number; d: number }[] {
  const p = ls.paths[i];
  return ls.frames.map(([, o], k) => ({ frame: o, d: p.distance[k] ?? 0 }));
}

/** The splits a measure takes part in, by the original's frame. */
export function splitsOf(ls: Lockstep, path: string): LockSplit[] {
  return ls.splits.filter((s) => s.paths.some((p) => p.path === path));
}

/** A measure's line under its name: how often together, and the farthest apart, in plain words. */
export function measureLine(p: LockPath): string {
  const unit = measureUnit(p.path);
  const pct = p.compared ? Math.round((100 * p.within) / p.compared) : 0;
  const together = `Together on ${p.within.toLocaleString("en-US")} of ${p.compared.toLocaleString("en-US")} frames (${pct}%).`;
  const far = unit.startsWith("apart") ? "" : p.max === 0 ? " Never apart." : ` Farthest apart: ${fmt(p.max)}${unit ? ` ${unit}` : ""}.`;
  const tol = p.tolerance > 0 ? ` Within ${fmt(p.tolerance)} counts as together.` : "";
  const free = p.level === "free" ? " The contract leaves it free." : "";
  return together + far + tol + free;
}

export function fmt(n: number): string {
  return Number.isInteger(n) ? String(n) : n.toFixed(n < 1 ? 3 : 1);
}

/** A part a translation couldn't carry over, in words. */
export function noteName(g: GameWords | undefined, path: string): string {
  const many = /^entities\.([a-z0-9_]+)\[\*\]$/.exec(path);
  if (many) return `the ${words(many[1])}s`;
  if (path === "rng") return "the random numbers";
  if (path === "(mechanic state)") return "timers and jump state";
  if (/^levels\.[^.]+\.tiles$/.test(path)) return "blocks hit and coins taken";
  if (path === "frame") return "the frame count";
  return measureName(g, path);
}

/** What a resync, or a start, couldn't load: the missing notes, in words. */
export function missingLine(g: GameWords | undefined, notes: LockNote[]): string {
  const missing = notes.filter((n) => n.how === "missing").map((n) => noteName(g, n.path));
  if (!missing.length) return "";
  const uniq = [...new Set(missing)];
  return `Not carried over, since the state mapping doesn't hold ${uniq.length === 1 ? "it" : "them"} yet: ${list(uniq)}.`;
}

/**
 * What a put-back resync couldn't load, said once: empty when the run
 * isn't put back, has no splits, or its missing list is the one the start's
 * line already says.
 */
export function resyncLine(g: GameWords | undefined, ls: Lockstep): string {
  if (ls.mode !== "resync" || !ls.splits.length) return "";
  const line = missingLine(g, ls.resync_notes);
  const names = (notes: LockNote[]) => [...new Set(notes.filter((n) => n.how === "missing").map((n) => noteName(g, n.path)))].sort().join("\n");
  return ls.start && names(ls.resync_notes) === names(ls.start.notes) ? "" : line;
}

export interface Bin {
  /** The bin's distances: over `from` up to `to`; the first bin is from 0 up to `to`, both included. */
  from: number;
  to: number;
  frames: number;
  /** Past the measure's tolerance: those frames are splits. */
  past: boolean;
  /** The last bin of a capped histogram: everything over `from`. */
  open?: boolean;
}

/**
 * A measure's spread: how many frames were each distance apart, counted
 * from the first measured frame to the original's frame `upTo`, so it
 * grows as the two games play. A yes/no measure, or a whole-number one
 * never more than 1 apart, has two bins, together and apart. A
 * whole-number measure never more than `most` apart has a bin a value.
 * Otherwise the first bin holds the frames within the tolerance (exactly
 * together when it is 0) and the rest double: 1/16 to 1/8 px, 1/8 to 1/4,
 * and on up to the run's largest, so a subpixel miss and a fall down a
 * pit both read, and the bins stay put while the count grows. At most
 * `cap` bins; the last holds everything past it.
 */
export function histogram(ls: Lockstep, i: number, upTo = Infinity, most = 10, cap = 14, since = -Infinity): Bin[] {
  const p = ls.paths[i];
  const ds = ls.frames.flatMap(([, o], k) => (o >= since && o <= upTo ? [p.distance[k] ?? 0] : []));
  const tol = p.tolerance;
  const top = Math.max(p.max, tol);
  if (twoBars(p, most)) {
    return [
      { from: 0, to: 0, frames: ds.filter((d) => d <= tol).length, past: false },
      { from: 0, to: 1, frames: ds.filter((d) => d > tol).length, past: true },
    ];
  }
  if (wholeSpread(p, most)) {
    const bins: Bin[] = Array.from({ length: top + 1 }, (_, v) => ({ from: v, to: v, frames: 0, past: v > tol }));
    for (const d of ds) bins[Math.min(top, Math.round(d))].frames++;
    return bins;
  }
  const bins: Bin[] = [{ from: 0, to: tol, frames: 0, past: false }];
  const past = p.distance.filter((d) => d > tol);
  if (past.length) {
    const small = Math.min(...past);
    // The doubling starts at the smallest miss, or just past the tolerance.
    let edge = tol > 0 ? 2 ** (Math.floor(Math.log2(tol)) + 1) : p.distance.every(Number.isInteger) ? 1 : Math.max(1 / 256, 2 ** Math.floor(Math.log2(small)));
    let from = tol;
    for (;;) {
      if (bins.length === cap - 1 && edge < top) {
        bins.push({ from, to: top, frames: 0, past: true, open: true });
        break;
      }
      bins.push({ from, to: edge, frames: 0, past: true });
      if (edge >= top) break;
      from = edge;
      edge *= 2;
    }
  }
  for (const d of ds) {
    const k = d <= tol ? 0 : bins.findIndex((b, j) => j > 0 && (b.open || d <= b.to));
    bins[k < 0 ? bins.length - 1 : k].frames++;
  }
  return bins;
}

/** Whether a measure's distances are whole numbers, never more than `most` apart: a bin a value. */
function wholeSpread(p: LockPath, most: number): boolean {
  return Math.max(p.max, p.tolerance) < most && Number.isInteger(p.tolerance) && p.distance.every(Number.isInteger);
}

/** Whether a measure's histogram is two bars, together and apart. */
function twoBars(p: LockPath, most: number): boolean {
  return measureUnit(p.path).startsWith("apart") || (wholeSpread(p, most) && p.max <= 1 && p.tolerance === 0);
}

/** A distance as a person reads it: a fraction of a pixel as 1/16, otherwise the number. */
function amount(n: number): string {
  if (n > 0 && n < 1 && Number.isInteger(Math.log2(n))) return `1/${1 / n}`;
  return fmt(n);
}

/** A bin's range in words: "together", "3 px", "within 2 px", "up to 1/16 px", "1/8 to 1/4 px", "more than 256 px". */
export function binName(b: Bin, p: LockPath, most = 10): string {
  if (twoBars(p, most)) return b.past ? "apart" : "together";
  const unit = measureUnit(p.path);
  const u = unit ? ` ${unit}` : "";
  if (wholeSpread(p, most)) return `${fmt(b.from)}${u}`;
  if (!b.past) return b.to === 0 ? "together" : `within ${amount(b.to)}${u}`;
  if (b.open) return `more than ${amount(b.from)}${u}`;
  const whole = p.distance.every(Number.isInteger);
  if (whole) return b.to - b.from === 1 ? `${fmt(b.to)}${u}` : `${fmt(Math.floor(b.from) + 1)} to ${fmt(b.to)}${u}`;
  return b.from === 0 ? `up to ${amount(b.to)}${u}` : `${amount(b.from)} to ${amount(b.to)}${u}`;
}

export interface Column {
  /** The original's frames the column covers, from and to, both included. */
  from: number;
  to: number;
  /** Frames measured in it. */
  frames: number;
  mean: number;
  max: number;
  /** Its splits for this measure: how many, and the first one's original frame. */
  splits: number;
  first?: number;
}

/**
 * A measure over the run in `n` columns of equal time, so a long run reads
 * at a glance: each column's average and largest distance and its splits.
 * Columns cover the original's frames `from` to `span`; one with no
 * measured frame is empty, and frames outside are left out.
 */
export function columns(ls: Lockstep, i: number, span: number, n = 100, from = 1): Column[] {
  const p = ls.paths[i];
  const len = Math.max(1, span - from + 1);
  const w = Math.max(1, len / n);
  const cols: Column[] = Array.from({ length: Math.min(n, Math.ceil(len / w)) }, (_, k) => ({
    from: Math.floor(from + k * w),
    to: Math.floor(from - 1 + (k + 1) * w),
    frames: 0,
    mean: 0,
    max: 0,
    splits: 0,
  }));
  const inside = (o: number) => o >= from && o <= span;
  const at = (o: number) => cols[Math.max(0, Math.min(cols.length - 1, Math.floor((o - from) / w)))];
  ls.frames.forEach(([, o], k) => {
    if (!inside(o)) return;
    const c = at(o);
    const d = p.distance[k] ?? 0;
    c.frames++;
    c.mean += d;
    c.max = Math.max(c.max, d);
  });
  for (const c of cols) if (c.frames) c.mean /= c.frames;
  for (const s of splitsOf(ls, p.path)) {
    if (!inside(s.original_frame)) continue;
    const c = at(s.original_frame);
    c.splits++;
    c.first = c.first === undefined ? s.original_frame : Math.min(c.first, s.original_frame);
  }
  return cols;
}

/**
 * A height from 0 to 1 for a distance on the doubling scale the histogram
 * uses: a subpixel miss shows, and a fall down a pit doesn't flatten the
 * rest. `small` is the smallest miss worth showing.
 */
export function scaled(d: number, top: number, small: number): number {
  if (d <= 0 || top <= 0) return 0;
  return Math.log2(1 + d / small) / Math.log2(1 + top / small);
}

/** The smallest miss on a measure, the doubling scale's step: at least 1/256. */
export function smallest(p: LockPath): number {
  const past = p.distance.filter((d) => d > 0);
  return past.length ? Math.max(1 / 256, Math.min(...past)) : 1;
}

/** A stretch of the original's frames the view is about, both included. */
export interface Focus {
  from: number;
  to: number;
}

/** A stretch's length and place in words: "5 seconds: 3.6 to 8.6 seconds". */
export function focusWords(f: Focus): string {
  const len = (f.to - f.from + 1) / NES_FPS;
  const n = Number.isInteger(Math.round(len * 10) / 10) ? Math.round(len) : Math.round(len * 10) / 10;
  return `${n === 1 ? "1 second" : `${n} seconds`}: ${(f.from / NES_FPS).toFixed(1)} to ${(f.to / NES_FPS).toFixed(1)} seconds`;
}

export interface Miss {
  path: string;
  /** Frames of the stretch past the measure's tolerance, of `compared`. */
  apart: number;
  compared: number;
  max: number;
  /** The first frame of the stretch it's apart on. */
  first: number;
}

/** The measures that don't match in a stretch, most often apart first. */
export function focusMisses(ls: Lockstep, f: Focus): Miss[] {
  const out: Miss[] = [];
  ls.paths.forEach((p) => {
    let apart = 0;
    let compared = 0;
    let max = 0;
    let first = Infinity;
    ls.frames.forEach(([, o], k) => {
      if (o < f.from || o > f.to) return;
      const d = p.distance[k] ?? 0;
      compared++;
      if (d > p.tolerance) {
        apart++;
        max = Math.max(max, d);
        first = Math.min(first, o);
      }
    });
    if (apart) out.push({ path: p.path, apart, compared, max, first });
  });
  return out.sort((a, b) => b.apart - a.apart || a.first - b.first);
}

/** A miss in words: "Mario's position across: apart on 280 of 300 frames, up to 3 px". */
export function missLine(g: GameWords | undefined, m: Miss): string {
  const unit = measureUnit(m.path);
  const up = unit.startsWith("apart") ? "" : `, up to ${fmt(m.max)}${unit ? ` ${unit}` : ""}`;
  const name = measureName(g, m.path);
  return `${name.charAt(0).toUpperCase()}${name.slice(1)}: apart on ${m.apart} of ${m.compared} frames${up}`;
}

/** Where the view looks first: a second before the earliest split, for `len` frames. */
export function firstFocus(ls: Lockstep, len: number): Focus | undefined {
  const firsts = ls.paths.flatMap((p) => (p.first_split ? [p.first_split.original_frame] : []));
  if (!firsts.length) return undefined;
  const from = Math.max(ls.frames[0]?.[1] ?? 1, Math.min(...firsts) - Math.round(NES_FPS));
  return { from, to: from + len - 1 };
}

/** A measure's line in a stretch: how often together there, and the farthest apart. */
export function rangeLine(ls: Lockstep, i: number, f: Focus): string {
  const p = ls.paths[i];
  let within = 0;
  let compared = 0;
  let max = 0;
  ls.frames.forEach(([, o], k) => {
    if (o < f.from || o > f.to) return;
    const d = p.distance[k] ?? 0;
    compared++;
    if (d <= p.tolerance) within++;
    max = Math.max(max, d);
  });
  return measureLine({ ...p, within, compared, max });
}

/**
 * Where the rebuilt game's start is, for the game view and a guide that
 * drives it (docs/inspector.md, "The same commands to both"): loading the
 * original's state, ready (started from it, with what wasn't carried
 * over), failed, or none (no rebuilt game). `says` is what is happening,
 * `next` what comes after.
 */
export interface StartPhase {
  state: "none" | "loading" | "ready" | "failed";
  says: string;
  next: string;
  /** The original's frame both started at, once ready. */
  frame?: number;
  /** What the translation couldn't carry over, in words. */
  missing?: string;
}

export function startPhase(
  g: GameWords | undefined,
  st: { original_frame: number; notes: LockNote[] } | undefined,
  frame: number,
  error?: string,
): StartPhase {
  if (!g?.preview) return { state: "none", says: "There is no rebuilt game to start.", next: "Start the run with --preview to build one." };
  if (error) return { state: "failed", says: `The rebuilt game couldn't start from the original's state: ${error}.`, next: "Nothing is compared until it can." };
  if (!st) return { state: "loading", says: "Loading the original's state into the rebuilt game...", next: "Then both play the same button presses from there." };
  const missing = missingLine(g, st.notes);
  const before = frame < st.original_frame ? ` The model's game begins at frame ${st.original_frame}, so until then it stands still.` : "";
  return {
    state: "ready",
    says: `Started from the original's state at frame ${st.original_frame} (${seconds(st.original_frame)}), and given the same button presses from there; it is never put back.${before}${missing ? ` ${missing}` : ""}`,
    next: "Play, or step, to compare the two.",
    frame: st.original_frame,
    missing: missing || undefined,
  };
}

/** A measure's drift threshold: the server's, else 2 px a position, 0.5 px a frame a speed, 1 anything else. */
export function driftThreshold(p: LockPath): number {
  if (p.drift !== undefined) return p.drift;
  const f = p.path.split(".").pop() ?? "";
  if (["x", "y", "stage_x", "stage_y"].includes(f)) return Math.max(2, p.tolerance);
  if (["vx", "vy"].includes(f)) return Math.max(0.5, p.tolerance);
  return Math.max(1, p.tolerance);
}

/** Where the two drift apart: a measure past its drift threshold. */
export interface DriftAt {
  frame: number;
  path: string;
  distance: number;
  threshold: number;
}

/**
 * The first of the original's frames from `from` to `to` on which a
 * measure has drifted past its threshold (docs/inspector.md, "Where they
 * drift apart"), the farthest past it named. Read from a free run, since
 * the rebuilt game the view shows is never put back.
 */
export function driftAt(ls: Lockstep, from: number, to = Infinity): DriftAt | undefined {
  for (let k = 0; k < ls.frames.length; k++) {
    const o = ls.frames[k][1];
    if (o < from || o > to) continue;
    let best: DriftAt | undefined;
    let worst = 1;
    for (const p of ls.paths) {
      const d = p.distance[k] ?? 0;
      const t = driftThreshold(p);
      if (d > t && d / Math.max(t, 1e-9) > worst) {
        worst = d / Math.max(t, 1e-9);
        best = { frame: o, path: p.path, distance: d, threshold: t };
      }
    }
    if (best) return best;
  }
  return undefined;
}

/** A drift in words: "They drifted apart here: Mario's height, 2.4 px (more than 2 px)". */
export function driftWords(g: GameWords | undefined, d: DriftAt): string {
  const unit = measureUnit(d.path);
  const u = unit && !unit.startsWith("apart") ? ` ${unit}` : "";
  const name = measureName(g, d.path);
  return `They drifted apart here, at ${seconds(d.frame)}: ${name}, ${fmt(d.distance)}${u} (more than ${fmt(d.threshold)}${u})`;
}
