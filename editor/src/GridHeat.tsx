// The grid heatmap under the thumbnails (docs/show.md, "The grid"): every
// cell of mechanics by contexts as one small square colored by its state,
// the needs-you ones amber. The pointer on a cell shows one line; a click
// opens the clip drawer for its clip, or says "empty: next up". The
// "clips in flight" stepper in the header sets how many clips the grid
// keeps in flight at once (`/api/capture/{run}/grid/in-flight`).

import { useEffect, useMemo, useRef, useState } from "react";
import type { Api } from "./api";
import type { BgState } from "./backfold";
import type { WorkState } from "./agentfold";
import { Stepper } from "./Calm";
import { needsYouClips } from "./calmfold";
import { type CellLook, type GridCell, type GridState, cellClick, cellLine, cellLook, gridSays, shownCols, shownRows } from "./gridfold";

const enc = encodeURIComponent;

/** The grid, polled every 5 seconds while the page is open. */
export function useGrid(api: Api, run: string) {
  const path = `/api/capture/${enc(run)}/grid`;
  const [state, setState] = useState<GridState>();
  useEffect(() => {
    let live = true;
    const load = () =>
      api
        .json<GridState>(path)
        .then((s) => live && setState(s))
        .catch(() => undefined);
    void load();
    const t = setInterval(load, 5000);
    return () => {
      live = false;
      clearInterval(t);
    };
  }, [api, path]);
  return state;
}

/** "clips in flight [-] N [+]": kept with the run. */
export function FlightControl({ api, run }: { api: Api; run: string }) {
  const path = `/api/capture/${enc(run)}/grid/in-flight`;
  const [state, setState] = useState<{ in_flight: number; max: number }>();
  useEffect(() => {
    let live = true;
    api
      .json<{ in_flight: number; max: number }>(path)
      .then((s) => live && setState(s))
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, [api, path]);
  const n = state?.in_flight ?? 3;
  return (
    <Stepper
      label="clips in flight"
      value={n}
      min={1}
      max={state?.max ?? n}
      set={(in_flight) =>
        void api
          .post<{ in_flight: number; max: number }>(path, { in_flight })
          .then(setState)
          .catch(() => undefined)
      }
      disabled={!state}
      testid="ctl-in-flight"
      title="How many clips the grid makes before one leaves: a new clip is made only when one in flight matches, is fixed, is finished by an agent or is skipped."
    />
  );
}

/** Each look's color on the canvas; the CSS legend uses the same. */
export const LOOK_COLOR: Record<CellLook, string> = {
  "no-start": "#16181e",
  empty: "#2c313c",
  next: "#2c313c",
  exact: "#3fb27f",
  waiting: "#4a5263",
  close: "#6cb6ff",
  differs: "#ff4d6d",
  fixing: "#b48cff",
  "needs-you": "#ffb020",
};

const CELL = 7;
const GAP = 1;
const STEP = CELL + GAP;
/** The gap between levels. */
const LEVEL_GAP = 3;

export function GridHeat({ grid, bg, work, open }: { grid?: GridState; bg?: BgState; work?: WorkState; open: (clip: string) => void }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const [all, setAll] = useState(false);
  const [line, setLine] = useState<string>();
  const cols = useMemo(() => (grid ? shownCols(grid, all) : []), [grid, all]);
  const rows = useMemo(() => (grid ? shownRows(grid, cols) : []), [grid, cols]);
  const needs = useMemo(() => needsYouClips(bg, work), [bg, work]);
  // Each column's x, a little apart at each new level.
  const xs = useMemo(() => {
    const out: number[] = [];
    let x = 0;
    cols.forEach((c, i) => {
      if (i > 0 && cols[i - 1].level !== c.level) x += LEVEL_GAP;
      out.push(x);
      x += STEP;
    });
    return out;
  }, [cols]);
  const width = (xs.length ? xs[xs.length - 1] + STEP : 0) + 1;
  const height = rows.length * STEP + 1;
  const cells = useMemo(() => {
    const m = new Map<string, GridCell>();
    for (const c of grid?.cells ?? []) m.set(`${c.row}|${c.col}`, c);
    return m;
  }, [grid]);
  const looks = useMemo(() => {
    const m = new Map<string, CellLook>();
    for (const [k, c] of cells) m.set(k, cellLook(c, bg, work, grid?.next, needs));
    return m;
  }, [cells, bg, work, grid, needs]);

  useEffect(() => {
    const cv = canvas.current;
    const g = cv?.getContext("2d");
    if (!cv || !g) return;
    g.clearRect(0, 0, cv.width, cv.height);
    rows.forEach((r, ri) => {
      cols.forEach((c, ci) => {
        const k = `${r.id}|${c.id}`;
        const look = looks.get(k);
        if (!look) return;
        const x = xs[ci];
        const y = ri * STEP;
        g.fillStyle = LOOK_COLOR[look];
        g.fillRect(x, y, CELL, CELL);
        if (look === "next") {
          g.strokeStyle = "#6cb6ff";
          g.strokeRect(x + 0.5, y + 0.5, CELL - 1, CELL - 1);
        }
      });
    });
  }, [rows, cols, xs, looks]);

  if (!grid) return null;
  if (grid.error)
    return (
      <p className="hint" data-testid="grid-error">
        The grid: {grid.error}
      </p>
    );
  if (!rows.length || !cols.length) return null;

  const at = (e: React.MouseEvent<HTMLCanvasElement>) => {
    const r = e.currentTarget.getBoundingClientRect();
    const x = e.clientX - r.left;
    const y = e.clientY - r.top;
    const ci = xs.findIndex((cx) => x >= cx && x < cx + CELL + GAP);
    const ri = Math.floor(y / STEP);
    if (ci < 0 || ri < 0 || ri >= rows.length) return undefined;
    const cell = cells.get(`${rows[ri].id}|${cols[ci].id}`);
    return cell ? { cell, row: rows[ri], col: cols[ci], look: looks.get(`${cell.row}|${cell.col}`) ?? "empty" } : undefined;
  };
  const flight = (grid.flight ?? []).map((f) => f.id);
  const counts = new Map<CellLook, number>();
  for (const l of looks.values()) counts.set(l, (counts.get(l) ?? 0) + 1);
  const legend: CellLook[] = ["exact", "close", "differs", "fixing", "needs-you", "waiting", "empty"];

  return (
    <section className="grid-heat" data-testid="grid-heat" aria-label="the grid of mechanics by contexts">
      <div className="thumbs-head">
        <span className="thumbs-title">The grid</span>
        <span className="hint" data-testid="grid-says">
          {gridSays(grid)}
        </span>
        {(counts.get("needs-you") ?? 0) > 0 && <span className="chip needs-you">{counts.get("needs-you")} need you</span>}
        <label className="hint" title="Also draw the places that have no start the rebuilt game can begin at yet">
          <input type="checkbox" checked={all} onChange={(e) => setAll(e.target.checked)} data-testid="grid-all" /> places with no start
        </label>
      </div>
      <div className="grid-scroll">
        <canvas
          ref={canvas}
          width={width}
          height={height}
          className="grid-canvas"
          data-testid="grid-canvas"
          onMouseMove={(e) => {
            const h = at(e);
            setLine(h ? cellLine(h.row, h.col, h.cell, h.look) : undefined);
          }}
          onMouseLeave={() => setLine(undefined)}
          onClick={(e) => {
            const h = at(e);
            if (!h) return;
            const c = cellClick(h.cell, h.look, flight);
            if ("open" in c) open(c.open);
            else setLine(`${cellLine(h.row, h.col, h.cell, h.look)}. ${c.says}`);
          }}
        />
      </div>
      <div className="grid-foot">
        <span className="grid-line one-line" data-testid="grid-line">
          {line ?? (grid.next ? `Next: ${grid.next.says}` : grid.says)}
        </span>
        <span className="grid-legend">
          {legend.map((l) => (
            <span key={l} className="grid-key">
              <span className="grid-swatch" style={{ background: LOOK_COLOR[l] }} />
              {l.replace("-", " ")}
            </span>
          ))}
        </span>
      </div>
    </section>
  );
}
