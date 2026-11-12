// The curve chart's side of the page (docs/show.md, "The background
// work"): one measure of a clip, the original's and the rebuilt game's
// values frame by frame, with the first wrong frame, the drift point and
// the value the background work is trying. Pure, so the tests run it on
// made-up curves.

/** One measure's values in a clip, from `GET .../background/curve`. */
export interface CurveMeasure {
  path: string;
  words: string;
  track: string;
  original: (number | null)[];
  model: (number | null)[];
  first_wrong?: number | null;
}

export interface ClipCurves {
  clip: string;
  model: string;
  /** The clip's frame of the first value. */
  offset: number;
  frames: number;
  tracks: { track: string; frame: number; path: string; says: string }[];
  drift?: { frame: number; path: string } | null;
  measures: CurveMeasure[];
}

/** The background job's parts the chart reads. */
export interface CurveJob {
  clip?: string;
  target?: { frame?: number; measures?: { path: string }[] } | null;
  trying?: { param?: string; value?: string; words?: string; pass?: string; curve?: { path?: string | null; frames?: number[]; values?: (number | null)[] } } | null;
}

/** A line's points: the clip's frame and the value. */
export type Points = [number, number][];

/** The measure the background work is working on in this clip, if it is. */
export function workingOn(job: CurveJob | null | undefined, clip: string): string | undefined {
  if (!job || job.clip !== clip) return undefined;
  return job.target?.measures?.[0]?.path ?? undefined;
}

/**
 * The measure to show: the one the background work is on in this clip,
 * else the first wrong track's measure, else the first that has numbers.
 */
export function defaultMeasure(c: ClipCurves | undefined, job: CurveJob | null | undefined): string | undefined {
  if (!c) return undefined;
  const has = (p: string) => c.measures.some((m) => m.path === p);
  const w = workingOn(job, c.clip);
  if (w && has(w)) return w;
  const t = c.tracks.find((x) => has(x.path));
  if (t) return t.path;
  return c.measures.find((m) => m.original.some((v) => v != null))?.path;
}

const ORDER = ["vx", "vy", "y", "x", "stage_x", "stage_y", "on_ground"];

/** The measures of the same entity (or the camera) as `path`, for the short list. */
export function sameEntity(c: ClipCurves | undefined, path: string | undefined): CurveMeasure[] {
  if (!c || !path) return [];
  const track = c.measures.find((m) => m.path === path)?.track;
  const rank = (p: string) => {
    const i = ORDER.indexOf(p.split(".").pop() ?? "");
    return i < 0 ? ORDER.length : i;
  };
  return c.measures
    .filter((m) => m.track === track && m.original.some((v) => v != null))
    .sort((a, b) => rank(a.path) - rank(b.path));
}

/** A series of values as points at the clip's frames, skipping the missing. */
export function points(values: (number | null)[] | undefined, offset: number, frames?: number[]): Points {
  const out: Points = [];
  (values ?? []).forEach((v, k) => {
    if (v != null) out.push([frames?.[k] ?? k + offset, v]);
  });
  return out;
}

/** The trial line for `path` in this clip, while the work is trying a value. */
export function tryingLine(job: CurveJob | null | undefined, clip: string, path: string | undefined): { points: Points; says: string } | undefined {
  const t = job?.trying;
  if (!job || job.clip !== clip || !t?.curve || !path || t.curve.path !== path || !t.curve.frames) return undefined;
  const pass = t.pass === "impact" ? " (finding which numbers matter)" : "";
  return { points: points(t.curve.values, 0, t.curve.frames), says: `trying ${t.words ?? t.param ?? "a number"} = ${t.value ?? "?"}${pass}` };
}

/** The bounds every line fits in, with a little room. */
export function bounds(lines: Points[], from: number, to: number): { x0: number; x1: number; y0: number; y1: number } {
  const ys = lines.flatMap((l) => l.filter(([x]) => x >= from && x <= to).map(([, y]) => y));
  let y0 = ys.length ? Math.min(...ys) : 0;
  let y1 = ys.length ? Math.max(...ys) : 1;
  if (y1 - y0 < 1e-9) {
    y0 -= 0.5;
    y1 += 0.5;
  }
  const pad = (y1 - y0) * 0.08;
  return { x0: from, x1: Math.max(to, from + 1), y0: y0 - pad, y1: y1 + pad };
}

/** An SVG polyline's points for a line in a `w` by `h` box. */
export function polyline(l: Points, b: { x0: number; x1: number; y0: number; y1: number }, w: number, h: number): string {
  return l
    .filter(([x]) => x >= b.x0 && x <= b.x1)
    .map(([x, y]) => `${(((x - b.x0) / (b.x1 - b.x0)) * w).toFixed(1)},${(h - ((y - b.y0) / (b.y1 - b.y0)) * h).toFixed(1)}`)
    .join(" ");
}

/** A frame's x in a `w` wide box. */
export function xAt(f: number, b: { x0: number; x1: number }, w: number): number {
  return ((f - b.x0) / (b.x1 - b.x0)) * w;
}
