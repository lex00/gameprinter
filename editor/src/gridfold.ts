// The grid heatmap's rules (docs/show.md, "The grid"): each cell's state
// on the page, the line that shows when the pointer is on it, what a click
// does, and which columns are drawn. The server gives each cell a state
// from the clips covering it; the page adds what it sees live, as the
// thumbnails do: a question for a person makes the cell "needs you", and a
// job or an agent on one of its clips makes it "being fixed". Pure, so the
// tests run it on made-up states.

import type { BgState } from "./backfold";
import type { WorkState } from "./agentfold";
import { agentOn, jobOn, needsYouClips } from "./calmfold";

/** A cell's state as the server says it. */
export type ServerState = "no_start" | "empty" | "exact" | "waiting" | "close" | "differs" | "fixing" | "needs_you";

/** A cell's state as the page draws it. */
export type CellLook = "no-start" | "empty" | "next" | "exact" | "waiting" | "close" | "differs" | "fixing" | "needs-you";

export interface GridRow {
  id: string;
  kind: "mechanic" | "option" | "entity" | "event";
  label: string;
  group: string;
}

export interface GridCol {
  id: string;
  level: string;
  level_name: string;
  place: string;
  starts: string[];
}

export interface GridCell {
  row: string;
  col: string;
  state: ServerState;
  clips: string[];
  doubt: number;
  priority?: number;
}

export interface GridNext {
  row: string;
  col: string;
  why: string;
  says: string;
  clip: string;
}

export interface GridState {
  run: string;
  mode: "grid" | "batch";
  in_flight: number;
  max_in_flight: number;
  made: number;
  left: number;
  says?: string;
  error?: string;
  rows?: GridRow[];
  cols?: GridCol[];
  cells?: GridCell[];
  names?: Record<string, string>;
  filled?: number;
  cell_count?: number;
  flight?: { id: string; row: string; col: string; why: string; says: string }[];
  next?: GridNext | null;
}

const SERVER: Record<ServerState, CellLook> = {
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
const RANK: CellLook[] = ["needs-you", "fixing", "differs", "close", "waiting", "exact", "next", "empty", "no-start"];

/** A cell's state on the page: the server's, then a question for you on one of its clips, or a job or agent on one. */
export function cellLook(cell: GridCell, bg: BgState | undefined, work: WorkState | undefined, next?: GridNext | null, needs = needsYouClips(bg, work)): CellLook {
  if (cell.clips.some((c) => needs.has(c))) return "needs-you";
  if (cell.clips.some((c) => jobOn(bg, c) || agentOn(bg, work, c))) return "fixing";
  const own = SERVER[cell.state] ?? "empty";
  if (own === "empty" && next && next.row === cell.row && next.col === cell.col) return "next";
  return own;
}

/** The worse of two looks. */
export function worse(a: CellLook, b: CellLook): CellLook {
  return RANK.indexOf(a) <= RANK.indexOf(b) ? a : b;
}

const WORDS: Record<CellLook, string> = {
  "no-start": "no start here the rebuilt game can begin at",
  empty: "empty",
  next: "empty: next up",
  exact: "matches",
  waiting: "waiting to be scored",
  close: "close",
  differs: "differs",
  fixing: "being fixed",
  "needs-you": "needs you",
};

/** The one line that shows when the pointer is on a cell: "jump, World 1-1 start: differs (2 clips)". */
export function cellLine(row: GridRow | undefined, col: GridCol | undefined, cell: GridCell, look: CellLook): string {
  const where = col ? `${col.level_name} ${col.place}` : cell.col;
  const n = cell.clips.length;
  const clips = n === 0 ? "" : n === 1 ? " (1 clip)" : ` (${n} clips)`;
  return `${row?.label ?? cell.row}, ${where}: ${WORDS[look]}${clips}`;
}

/** What a click on a cell does: open its clip (the one most in need first), or say it is empty. */
export function cellClick(cell: GridCell, look: CellLook, flight: string[] = []): { open: string } | { says: string } {
  if (!cell.clips.length) return { says: look === "no-start" ? WORDS["no-start"] : "empty: next up" };
  const first = cell.clips.find((c) => flight.includes(c)) ?? cell.clips[0];
  return { open: first };
}

/** The columns drawn: those with a start the rebuilt game can begin at, or every one. */
export function shownCols(g: GridState, all: boolean): GridCol[] {
  const cols = g.cols ?? [];
  if (all) return cols;
  const covered = new Set((g.cells ?? []).filter((c) => c.clips.length).map((c) => c.col));
  return cols.filter((c) => c.starts.length > 0 || covered.has(c.id));
}

/** The rows drawn: those with a cell in a shown column. */
export function shownRows(g: GridState, cols: GridCol[]): GridRow[] {
  const ids = new Set(cols.map((c) => c.id));
  const has = new Set((g.cells ?? []).filter((c) => ids.has(c.col)).map((c) => c.row));
  return (g.rows ?? []).filter((r) => has.has(r.id));
}

/** The counts under the grid: "140 of 5,800 cells filled; 3 in flight, at most 3". */
export function gridSays(g: GridState): string {
  const n = (x: number) => x.toLocaleString("en-US");
  const fly = g.flight?.length ?? 0;
  return `${n(g.filled ?? 0)} of ${n(g.cell_count ?? 0)} cells filled; ${fly} in flight, at most ${g.in_flight}.`;
}
