// The lockstep panel under the two games (docs/inspector.md, "The game
// view"): the original and the model on the same button presses, measured
// apart on every mapped measure every frame. A small chart a measure shows
// the distance over the run in about 100 columns (average and largest, on
// a doubling scale), with a red tick under each column with a split that
// jumps both games there; a toggle puts the model back at every split (resync)
// or lets the two drift (free run); one line says what agrees and what
// splits first. Beside each chart, a histogram of the measure's distances
// up to the view's frame, so it grows as the two play. The words come from
// lockfold.ts, filled from the model.

import { useEffect, useMemo, useState } from "react";
import type { Api, GameWords } from "./api";
import {
  type Focus,
  type LockMode,
  type LockPath,
  type Lockstep,
  binName,
  columns,
  fmt,
  histogram,
  lockSummary,
  measureLine,
  rangeLine,
  measureName,
  measureUnit,
  mechanicName,
  missingLine,
  scaled,
  seconds,
  smallest,
} from "./lockfold";

const CW = 600;
const CH = 64;
const PAD = 4;

export interface LockstepPanelProps {
  api: Api;
  run: string;
  words?: GameWords;
  recording?: string;
  /** The view's frame, the original's. */
  frame: number;
  /** Frames in the recording's input. */
  total: number;
  paused: boolean;
  seek: (frame: number) => void;
  /** The stretch the view is about: the charts and histograms zoom to it. */
  focus?: Focus;
  /** The lockstep measured, for the view's focus line. */
  onResult?: (ls: Lockstep | undefined) => void;
  /**
   * The original's frame both games start at, from its state: the view's
   * start, so the numbers are about what the two pictures show
   * (docs/inspector.md, "The same commands to both").
   */
  from?: number;
  /** Start both games at this frame of the original: the view moves its start there. */
  startHere?: (frame: number) => void;
  /** The frame the two drift apart, marked on each chart. */
  drift?: number;
}

export function LockstepPanel({ api, run, words: g, recording, frame, total, paused, seek, focus, onResult, from, startHere, drift }: LockstepPanelProps) {
  const [mode, setMode] = useState<LockMode>("resync");
  const [ls, setLs] = useState<Lockstep>();
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!recording) return;
    let live = true;
    setBusy(true);
    setError(undefined);
    api
      .captureLockstep(run, recording, mode, from)
      .then((j) => live && setLs(j))
      .catch((e: Error) => live && setError(e.message))
      .finally(() => live && setBusy(false));
    return () => {
      live = false;
    };
  }, [api, run, recording, mode, from]);

  useEffect(() => onResult?.(ls), [ls]); // eslint-disable-line react-hooks/exhaustive-deps
  const span = Math.max(total, ls?.frames.at(-1)?.[1] ?? 1);
  if (!recording) return null;
  return (
    <section className="lockstep" data-testid="lockstep">
      <h2>The two games, measured apart</h2>
      <div className="lockstep-controls">
        <label title="At each split the model takes the original's state and goes on, so each split is a new place the model parts">
          <input type="radio" name="lock-mode" checked={mode === "resync"} onChange={() => setMode("resync")} data-testid="lock-resync" /> put the model back at each split
        </label>
        <label title="The model is never put back: the distances show how far it drifts">
          <input type="radio" name="lock-mode" checked={mode === "free"} onChange={() => setMode("free")} data-testid="lock-free" /> let them drift
        </label>
        {paused && startHere && (
          <button onClick={() => startHere(frame)} data-testid="lock-start-here" title="The model takes the original's state at this frame, and both go on from it">
            start both here
          </button>
        )}
        {busy && <span className="hint">measuring...</span>}
      </div>
      {error && <p className="error">{error}</p>}
      {ls && <LockstepResult g={g} ls={ls} span={span} frame={frame} seek={seek} focus={focus} drift={drift} />}
    </section>
  );
}

/** A lockstep's summary, its start, a chart a measure and the splits by mechanic. */
export function LockstepResult({ g, ls, span, frame, seek, focus, drift }: { g?: GameWords; ls: Lockstep; span: number; frame: number; seek: (f: number) => void; focus?: Focus; drift?: number }) {
  return (
    <>
      <p className="lockstep-summary" data-testid="lock-summary">
        {lockSummary(g, ls)}
      </p>
      {ls.start && (
        <p className="hint" data-testid="lock-start">
          Both started at {seconds(ls.start.original_frame)} (frame {ls.start.original_frame}) from the original's state. {missingLine(g, ls.start.notes)}
        </p>
      )}
      {ls.mode === "resync" && ls.splits.length > 0 && <p className="hint">{missingLine(g, ls.resync_notes)}</p>}
      <div className="lock-charts">
        {ls.paths.map((p, i) => (
          <MeasureChart key={p.path} g={g} ls={ls} p={p} i={i} span={span} frame={frame} seek={seek} focus={focus} drift={drift} />
        ))}
      </div>
      {ls.mechanics.length > 0 && (
        <div className="lock-mechanics" data-testid="lock-mechanics">
          <h3>Where the model parts, by mechanic</h3>
          <ul>
            {ls.mechanics.map((m) => (
              <li key={m.mechanic}>
                {mechanicName(g, m.mechanic)}: {m.splits === 1 ? "one split" : `${m.splits} splits`}, in {m.paths.map((x) => measureName(g, x)).join(", ")}; first at {seconds(m.first.original_frame)}{" "}
                <button onClick={() => seek(m.first.original_frame)}>go there</button>
              </li>
            ))}
          </ul>
        </div>
      )}
    </>
  );
}

function MeasureChart({ g, ls, p, i, span, frame, seek, focus, drift }: { g?: GameWords; ls: Lockstep; p: LockPath; i: number; span: number; frame: number; seek: (f: number) => void; focus?: Focus; drift?: number }) {
  const [hover, setHover] = useState<number>();
  // Zoomed to the focus when there is one: a column a few frames.
  const lo = focus?.from ?? 1;
  const hi = focus?.to ?? span;
  const cols = useMemo(() => columns(ls, i, hi, Math.min(100, hi - lo + 1), lo), [ls, i, lo, hi]);
  const small = useMemo(() => smallest(p), [p]);
  const top = Math.max(p.max, p.tolerance, small);
  const cw = (CW - 2 * PAD) / Math.max(1, cols.length);
  const base = CH - PAD - 8;
  const h = (d: number) => scaled(d, top, small) * (base - PAD);
  const x = (f: number) => PAD + ((f - lo) / Math.max(1, hi - lo)) * (CW - 2 * PAD);
  const name = measureName(g, p.path);
  const unit = measureUnit(p.path);
  const u = unit && !unit.startsWith("apart") ? ` ${unit}` : "";
  const c = hover === undefined ? undefined : cols[hover];
  return (
    <figure className="lock-chart" data-testid="lock-chart">
      <figcaption>
        <b>{name.charAt(0).toUpperCase() + name.slice(1)}</b>
        <div className="lock-says">{focus ? rangeLine(ls, i, focus) : measureLine(p)}</div>
      </figcaption>
      <div className="lock-row">
        <svg viewBox={`0 0 ${CW} ${CH}`} preserveAspectRatio="none" width="100%" height={CH} onMouseLeave={() => setHover(undefined)} role="img" aria-label={`${name}: how far apart over the run`}>
          <line x1={PAD} x2={CW - PAD} y1={base} y2={base} className="lock-axis" />
          {p.tolerance > 0 && <line x1={PAD} x2={CW - PAD} y1={base - h(p.tolerance)} y2={base - h(p.tolerance)} className="lock-tolerance" />}
          {cols.map((k, n) => (
            <g key={n} className={hover === n ? "lock-col hover" : "lock-col"} onMouseEnter={() => setHover(n)} onClick={() => seek(k.first ?? k.from)} role="button" aria-label={`${seconds(k.from)} to ${seconds(k.to)}`}>
              <rect x={PAD + n * cw} y={0} width={cw} height={CH} className="lock-hit" />
              {k.max > p.tolerance && <rect x={PAD + n * cw + 0.5} y={base - h(k.max)} width={Math.max(0.5, cw - 1)} height={h(k.max)} className="lock-col-max" />}
              {k.mean > 0 && <rect x={PAD + n * cw + 0.5} y={base - h(k.mean)} width={Math.max(0.5, cw - 1)} height={h(k.mean)} className={k.mean > p.tolerance ? "lock-col-mean past" : "lock-col-mean"} />}
              {k.splits > 0 && <rect x={PAD + n * cw + 0.5} y={base + 2} width={Math.max(0.5, cw - 1)} height={5} className="lock-col-split" />}
            </g>
          ))}
          {drift !== undefined && drift >= lo && drift <= hi && <line x1={x(drift)} x2={x(drift)} y1={0} y2={CH} className="lock-drift" />}
          {frame >= lo && frame <= hi && <line x1={x(frame)} x2={x(frame)} y1={0} y2={CH} className="lock-playhead" />}
        </svg>
        <Histogram ls={ls} p={p} i={i} frame={frame} name={name} since={focus?.from} />
      </div>
      {c && (
        <div className="lock-tip hint" data-testid="lock-tip">
          {seconds(c.from)} to {seconds(c.to)}:{" "}
          {c.frames ? `on average ${fmt(c.mean)}${u} apart, at most ${fmt(c.max)}${u}` : "not measured"}
          {c.splits ? `; ${c.splits === 1 ? "one split" : `${c.splits} splits`}, click to go to the first` : ""}
        </div>
      )}
    </figure>
  );
}

/** A measure's distances up to the view's frame, a labelled bar a distance: within the tolerance plain, past it red. */
export function Histogram({ ls, p, i, frame, name, since }: { ls: Lockstep; p: LockPath; i: number; frame: number; name: string; since?: number }) {
  const bins = useMemo(() => histogram(ls, i, frame, undefined, undefined, since), [ls, i, frame, since]);
  const most = Math.max(1, ...bins.map((b) => b.frames));
  const total = bins.reduce((a, b) => a + b.frames, 0);
  return (
    <div className="lock-histogram" role="img" aria-label={`${name}: how many frames were each distance apart, up to this frame`} data-testid="lock-histogram">
      <div className="hint">
        How far apart, frame by frame, {since === undefined ? "from the start" : "from the start of the stretch"} up to this frame ({total.toLocaleString("en-US")} frames)
      </div>
      {bins.map((b, k) => (
        <div key={k} className={b.past ? "lock-hrow past" : "lock-hrow"}>
          <span className="lock-hlabel">{binName(b, p)}</span>
          <span className="lock-hbar">
            <span style={{ width: `${(100 * b.frames) / most}%` }} />
          </span>
          <span className="lock-hcount">
            {b.frames.toLocaleString("en-US")}
            {total ? ` (${Math.round((100 * b.frames) / total)}%)` : ""}
          </span>
        </div>
      ))}
    </div>
  );
}
