// REFINE's grid (game-printer's editor/src/GridHeat.tsx with its rules from
// gridfold.ts; game-printer's docs/show.md, "The grid"): every cell of
// mechanics by contexts as one small square colored by its state, the
// needs-you ones amber. The pointer on a cell shows one line; a click
// opens its clip on CLIPS, or says "empty: next up". The "clips in flight"
// stepper sets how many clips the grid keeps in flight
// (/api/capture/{run}/grid/in-flight). The grid comes from gp serve
// (crates/gp-cli/src/serve/grid.rs), read again every 5 seconds; the
// background work's state adds what it sees live, as the cards do: a job
// on one of a cell's clips makes it "being fixed".

import { css, h, hub, openClip, opts } from "./fold.js";

const enc = encodeURIComponent;

/** A cell's state as the server says it, as the page draws it. */
const SERVER = {
  no_start: "no-start",
  empty: "empty",
  exact: "exact",
  waiting: "waiting",
  close: "close",
  differs: "differs",
  fixing: "fixing",
  needs_you: "needs-you",
};

/** Worst first: the state a cell shows when its clips stand differently. */
const RANK = ["needs-you", "fixing", "differs", "close", "waiting", "exact", "next", "empty", "no-start"];

const WORDS = {
  "no-start": "no clip starts here yet",
  empty: "empty",
  next: "empty: next up",
  exact: "matches",
  waiting: "waiting to be scored",
  close: "close",
  differs: "differs",
  fixing: "being fixed",
  "needs-you": "needs you",
};

/** Each look's color on the canvas; the legend uses the same. */
export const LOOK_COLOR = {
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

/** The clips a job has now, from the background work's state. */
export function workingOn(bg) {
  return new Set((bg?.jobs ?? []).map((j) => j.clip).filter(Boolean));
}

/** A cell's state on the page: the server's, then a job on one of its clips. */
export function cellLook(cell, working, next) {
  const own = SERVER[cell.state] ?? "empty";
  if (own !== "needs-you" && cell.clips.some((c) => working.has(c)) && (own === "differs" || own === "close")) return "fixing";
  if (own === "empty" && next && next.row === cell.row && next.col === cell.col) return "next";
  return own;
}

/** The worse of two looks. */
export function worse(a, b) {
  return RANK.indexOf(a) <= RANK.indexOf(b) ? a : b;
}

/** The one line that shows when the pointer is on a cell: "jump, 25m jump: differs (2 clips)". */
export function cellLine(row, col, cell, look) {
  const where = col ? `${col.level_name} ${col.place}` : cell.col;
  const n = cell.clips.length;
  const clips = n === 0 ? "" : n === 1 ? " (1 clip)" : ` (${n} clips)`;
  return `${row?.label ?? cell.row}, ${where}: ${WORDS[look]}${clips}`;
}

/** What a click on a cell does: open its clip (one in flight first), or say it is empty. */
export function cellClick(cell, look, flight = []) {
  if (!cell.clips.length) return { says: look === "no-start" ? WORDS["no-start"] : "empty: next up" };
  return { open: cell.clips.find((c) => flight.includes(c)) ?? cell.clips[0] };
}

/** The columns drawn: those with a clip starting there, or every one. */
export function shownCols(g, all) {
  const cols = g.cols ?? [];
  if (all) return cols;
  const covered = new Set((g.cells ?? []).filter((c) => c.clips.length).map((c) => c.col));
  return cols.filter((c) => c.starts.length > 0 || covered.has(c.id));
}

/** The rows drawn: those with a cell in a shown column. */
export function shownRows(g, cols) {
  const ids = new Set(cols.map((c) => c.id));
  const has = new Set((g.cells ?? []).filter((c) => ids.has(c.col)).map((c) => c.row));
  return (g.rows ?? []).filter((r) => has.has(r.id));
}

/** The counts beside the title: "40 of 580 cells filled; 0 in flight, at most 3." */
export function gridSays(g) {
  const n = (x) => x.toLocaleString("en-US");
  return `${n(g.filled ?? 0)} of ${n(g.cell_count ?? 0)} cells filled; ${g.flight?.length ?? 0} in flight, at most ${g.in_flight}.`;
}

const CELL = 7;
const GAP = 1;
const STEP = CELL + GAP;
/** The gap between levels. */
const LEVEL_GAP = 3;

/** Mounts the grid on `el`; returns {stop()}. */
export function refineGrid(el, a, b, c) {
  css();
  const o = opts(a, b, c);
  const x = hub(o);
  el.classList.add("rf", "rf-grid");
  el.dataset.testid = "grid-heat";
  const base = `/api/capture/${enc(o.run)}/grid`;
  let grid = null;
  let flight = null;
  let all = false;
  let line = null;

  const load = async () => {
    try {
      grid = await o.api(base);
    } catch (e) {
      grid = { error: String(e.message ?? e) };
    }
    draw();
  };
  const loadFlight = async (body) => {
    try {
      flight = await o.api(`${base}/in-flight`, body);
    } catch {
      flight = null;
    }
    draw();
  };

  function draw() {
    el.textContent = "";
    const head = h("div", "rf-head");
    head.append(h("h3", "", "The grid"));
    el.append(head);
    if (!grid) {
      el.append(h("p", "rf-muted", "Reading the grid."));
      return;
    }
    if (grid.error) {
      const p = h("p", "rf-muted", `The grid: ${grid.error}`);
      p.dataset.testid = "grid-error";
      el.append(p);
      return;
    }
    const cols = shownCols(grid, all);
    const rows = shownRows(grid, cols);
    const working = workingOn(x.state);
    const cells = new Map();
    for (const cl of grid.cells ?? []) cells.set(`${cl.row}|${cl.col}`, cl);
    const looks = new Map();
    for (const [k, cl] of cells) looks.set(k, cellLook(cl, working, grid.next));
    const counts = new Map();
    for (const l of looks.values()) counts.set(l, (counts.get(l) ?? 0) + 1);

    const says = h("span", "rf-says", gridSays(grid));
    says.dataset.testid = "grid-says";
    head.append(says);
    if ((counts.get("needs-you") ?? 0) > 0) head.append(h("span", "rf-chip rf-needs-you", `${counts.get("needs-you")} need you`));
    const lab = h("label", "rf-muted");
    lab.title = "Also draw the places no clip starts at yet";
    const box = h("input");
    box.type = "checkbox";
    box.checked = all;
    box.dataset.testid = "grid-all";
    box.addEventListener("change", () => { all = box.checked; draw(); });
    lab.append(box, document.createTextNode(" places with no clip"));
    head.append(lab, stepper());

    if (!rows.length || !cols.length) {
      el.append(h("p", "rf-muted", grid.says ?? "No cell yet: the game's corpus has no clip."));
      return;
    }
    // Each column's x, a little apart at each new level.
    const xs = [];
    let at = 0;
    cols.forEach((col, i) => {
      if (i > 0 && cols[i - 1].level !== col.level) at += LEVEL_GAP;
      xs.push(at);
      at += STEP;
    });
    const scroll = h("div", "rf-grid-scroll");
    const cv = h("canvas", "rf-grid-canvas");
    cv.dataset.testid = "grid-canvas";
    cv.width = (xs.length ? xs[xs.length - 1] + STEP : 0) + 1;
    cv.height = rows.length * STEP + 1;
    cv.setAttribute("aria-label", "the grid of mechanics by contexts");
    const g = cv.getContext("2d");
    if (g) {
      rows.forEach((r, ri) => {
        cols.forEach((col, ci) => {
          const look = looks.get(`${r.id}|${col.id}`);
          if (!look) return;
          const cx = xs[ci];
          const cy = ri * STEP;
          g.fillStyle = LOOK_COLOR[look];
          g.fillRect(cx, cy, CELL, CELL);
          if (look === "next") {
            g.strokeStyle = "#6cb6ff";
            g.strokeRect(cx + 0.5, cy + 0.5, CELL - 1, CELL - 1);
          }
        });
      });
    }
    const hit = (e) => {
      const rc = cv.getBoundingClientRect();
      const px = (e.clientX - rc.left) * (cv.width / rc.width);
      const py = (e.clientY - rc.top) * (cv.height / rc.height);
      const ci = xs.findIndex((cx) => px >= cx && px < cx + STEP);
      const ri = Math.floor(py / STEP);
      if (ci < 0 || ri < 0 || ri >= rows.length) return null;
      const cell = cells.get(`${rows[ri].id}|${cols[ci].id}`);
      return cell ? { cell, row: rows[ri], col: cols[ci], look: looks.get(`${cell.row}|${cell.col}`) ?? "empty" } : null;
    };
    const foot = h("div", "rf-grid-foot");
    const ln = h("span", "rf-grid-line", line ?? (grid.next ? `Next: ${grid.next.says}` : grid.says ?? ""));
    ln.dataset.testid = "grid-line";
    cv.addEventListener("mousemove", (e) => {
      const t = hit(e);
      ln.textContent = t ? cellLine(t.row, t.col, t.cell, t.look) : (grid.next ? `Next: ${grid.next.says}` : grid.says ?? "");
    });
    cv.addEventListener("click", (e) => {
      const t = hit(e);
      if (!t) return;
      const r = cellClick(t.cell, t.look, (grid.flight ?? []).map((f) => f.id));
      if (r.open) openClip(o.run, r.open);
      else {
        line = `${cellLine(t.row, t.col, t.cell, t.look)}. ${r.says}`;
        ln.textContent = line;
      }
    });
    scroll.append(cv);
    el.append(scroll);
    const legend = h("span", "rf-grid-legend");
    for (const l of ["exact", "close", "differs", "fixing", "needs-you", "waiting", "empty"]) {
      const k = h("span", "rf-grid-key");
      const sw = h("span", "rf-grid-swatch");
      sw.style.background = LOOK_COLOR[l];
      k.append(sw, document.createTextNode(l.replace("-", " ")));
      legend.append(k);
    }
    foot.append(ln, legend);
    el.append(foot);
  }

  /** "clips in flight [-] N [+]": kept with the run. */
  function stepper() {
    const w = h("span", "rf-stepper");
    w.dataset.testid = "ctl-in-flight";
    w.title = "How many clips the grid keeps in flight. gameprinter's grid picks the next cell and says it; the clips themselves come from the mechanic sessions";
    const n = flight?.in_flight ?? grid?.in_flight ?? 3;
    const max = flight?.max ?? grid?.max_in_flight ?? n;
    const minus = h("button", "", "-");
    const plus = h("button", "", "+");
    minus.disabled = !flight || n <= 1;
    plus.disabled = !flight || n >= max;
    minus.addEventListener("click", () => void loadFlight({ in_flight: n - 1 }));
    plus.addEventListener("click", () => void loadFlight({ in_flight: n + 1 }));
    w.append(h("span", "rf-muted", "clips in flight "), minus, h("span", "rf-n", String(n)), plus);
    return w;
  }

  const off = x.subscribe((_, why) => { if (why === "state") draw(); });
  void load();
  void loadFlight();
  const t = setInterval(() => void load(), 5000);
  return {
    stop() {
      clearInterval(t);
      off();
      x.stop();
      el.textContent = "";
    },
  };
}

export default refineGrid;
