// The curve chart (docs/show.md, "The background work"): one measure,
// the original's and the rebuilt game's values frame by frame, as two
// lines with the playhead, the first wrong frame and the drift point. In
// the clip card it follows the measure the background work is on and
// shows the value being tried as a third, fainter line; in the game view
// it reads the lockstep; in "what it did" it shows a round's before and
// after.

import { useEffect, useMemo, useState } from "react";
import type { Api } from "./api";
import type { Lockstep } from "./lockfold";
import { type ClipCurves, type CurveJob, type Points, bounds, defaultMeasure, points, polyline, sameEntity, tryingLine, workingOn, xAt } from "./curvefold";

const W = 320;
const H = 90;

const FIELD_WORDS: Record<string, string> = {
  vx: "speed across",
  vy: "speed up and down",
  y: "height",
  x: "position",
  stage_x: "position",
  stage_y: "height",
};

function fieldWords(path: string, words?: string): string {
  return FIELD_WORDS[path.split(".").pop() ?? ""] ?? words ?? path;
}

export interface CurveLinesProps {
  title: string;
  original: Points;
  rebuilt: Points;
  /** A third, fainter line: a value being tried, or the rebuilt game before a change. */
  third?: { points: Points; says: string };
  rebuiltLabel?: string;
  from: number;
  to: number;
  playhead?: number;
  firstWrong?: number | null;
  drift?: number | null;
  picker?: React.ReactNode;
}

/** The chart itself: two or three lines in a small box. */
export function CurveLines({ title, original, rebuilt, third, rebuiltLabel = "model", from, to, playhead, firstWrong, drift, picker }: CurveLinesProps) {
  const b = bounds([original, rebuilt, ...(third ? [third.points] : [])], from, to);
  const mark = (f: number | null | undefined, cls: string, label: string) =>
    f != null && f >= b.x0 && f <= b.x1 ? (
      <line className={cls} x1={xAt(f, b, W)} x2={xAt(f, b, W)} y1={0} y2={H}>
        <title>{label}</title>
      </line>
    ) : null;
  return (
    <figure className="curve-chart" data-testid="curve-chart">
      <figcaption>
        {title}
        {picker}
      </figcaption>
      <svg viewBox={`0 0 ${W} ${H}`} width={W} height={H} role="img" aria-label={title}>
        {mark(firstWrong, "curve-first", `first wrong frame: ${firstWrong}`)}
        {mark(drift, "curve-drift", `drift point: ${drift}`)}
        {third && <polyline className="curve-third" points={polyline(third.points, b, W, H)} />}
        <polyline className="curve-original" points={polyline(original, b, W, H)} />
        <polyline className="curve-rebuilt" points={polyline(rebuilt, b, W, H)} />
        {mark(playhead, "curve-playhead", `frame ${playhead}`)}
      </svg>
      <p className="curve-key">
        <span className="key-original">original</span> <span className="key-rebuilt">{rebuiltLabel}</span>
        {third && <span className="key-third">{third.says}</span>}
        {firstWrong != null && <span className="key-first">wrong from frame {firstWrong}</span>}
        {drift != null && <span className="key-drift">drifts at frame {drift}</span>}
      </p>
    </figure>
  );
}

/** The clip card's chart: the clip's curves from the background work, following what it is working on. */
export function ClipCurve({ api, run, clip, at, job, model, attempt }: { api: Api; run: string; clip: string; at: number; job?: CurveJob | null; model?: string; attempt?: { n: number; path: string } }) {
  const [c, setC] = useState<ClipCurves & { before?: ClipCurves | null }>();
  const [picked, setPicked] = useState<{ path: string; following?: string }>();
  useEffect(() => {
    let live = true;
    api
      .json<ClipCurves>(attempt?.path ?? `/api/capture/${encodeURIComponent(run)}/background/curve?clip=${encodeURIComponent(clip)}`)
      .then((v) => live && setC(v))
      .catch(() => live && setC(undefined));
    return () => {
      live = false;
    };
  }, [api, run, clip, model, attempt?.path]);
  const working = workingOn(job, clip);
  // The person's pick holds until the work moves on to another measure.
  const path = picked && picked.following === working ? picked.path : defaultMeasure(c, job);
  const list = useMemo(() => sameEntity(c, path), [c, path]);
  const m = c?.measures.find((x) => x.path === path);
  if (!c || !m) return null;
  const off = c.offset ?? 1;
  const picker =
    list.length > 1 ? (
      <select value={m.path} onChange={(e) => setPicked({ path: e.target.value, following: working })} data-testid="curve-pick">
        {list.map((x) => (
          <option key={x.path} value={x.path}>
            {fieldWords(x.path, x.words)}
          </option>
        ))}
      </select>
    ) : null;
  const who = m.words.split("'s ")[0];
  // An attempt's curve: the attempt as the rebuilt line, the game as it is now as the third.
  const was = attempt ? c.before?.measures.find((x) => x.path === m.path) : undefined;
  const third = attempt ? (was ? { points: points(was.model, c.before?.offset ?? off), says: "as it is now" } : undefined) : tryingLine(job, clip, m.path);
  return (
    <CurveLines
      title={`${who}'s ${fieldWords(m.path, m.words)}, original vs rebuilt, frame by frame `}
      original={points(m.original, off)}
      rebuilt={points(m.model, off)}
      third={third}
      rebuiltLabel={attempt ? `attempt ${attempt.n}` : undefined}
      from={off}
      to={off + Math.max(m.original.length, 1) - 1}
      playhead={at}
      firstWrong={m.first_wrong}
      drift={c.drift?.path === m.path || !m.first_wrong ? c.drift?.frame : undefined}
      picker={picker}
    />
  );
}

/** The game view's chart: one measure of its lockstep around the playhead. */
export function LockCurve({ ls, frame, span = 300 }: { ls?: Lockstep; frame: number; span?: number }) {
  type P = Lockstep["paths"][number] & { original?: (number | null)[]; model?: (number | null)[] };
  const paths = ((ls?.paths ?? []) as P[]).filter((p) => p.original?.some((v) => v != null));
  const first = paths.filter((p) => p.first_split).sort((a, b) => a.first_split!.original_frame - b.first_split!.original_frame)[0];
  const [pick, setPick] = useState<string>();
  const p = paths.find((x) => x.path === pick) ?? first ?? paths[0];
  if (!ls || !p) return null;
  const frames = ls.frames.map(([, o]) => o);
  const from = Math.max(frames[0] ?? 0, frame - span / 2);
  const to = from + span;
  const owner = p.path.startsWith("cameras.") ? "the camera" : (p.path.split(".")[1]?.split("[")[0] ?? "");
  const name = owner.charAt(0).toUpperCase() + owner.slice(1);
  const kind = (x: string) => x.split(".").slice(0, 2).join(".");
  const list = paths.filter((x) => kind(x.path) === kind(p.path));
  return (
    <CurveLines
      title={`${name}'s ${fieldWords(p.path)}, original vs rebuilt, frame by frame `}
      original={points(p.original, 0, frames)}
      rebuilt={points(p.model, 0, frames)}
      from={from}
      to={to}
      playhead={frame}
      firstWrong={p.first_split?.original_frame}
      picker={
        list.length > 1 ? (
          <select value={p.path} onChange={(e) => setPick(e.target.value)} data-testid="curve-pick">
            {list.map((x) => (
              <option key={x.path} value={x.path}>
                {fieldWords(x.path)}
              </option>
            ))}
          </select>
        ) : null
      }
    />
  );
}

/** A round's curve in "what it did": the measure it worked on, before and after. */
export function RoundCurve({ curve }: { curve?: { path: string; words: string; original?: string[] | null; before?: string[] | null; after?: string[] | null } | null }) {
  const [open, setOpen] = useState(false);
  if (!curve?.original?.length) return null;
  const num = (v?: string[] | null) => (v ?? []).map((x) => (x === "" ? null : Number(x)));
  const o = num(curve.original);
  return (
    <>
      {" "}
      <button className="link" onClick={() => setOpen(!open)} data-testid="round-curve-toggle">
        {open ? "hide curve" : "curve"}
      </button>
      {open && (
        <CurveLines
          title={`${curve.words}, before and after `}
          original={points(o, 1)}
          rebuilt={points(num(curve.after), 1)}
          rebuiltLabel="after"
          third={{ points: points(num(curve.before), 1), says: "before" }}
          from={1}
          to={o.length}
        />
      )}
    </>
  );
}
