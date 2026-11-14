// A capture run's events folded into what the live panel draws: each
// stage's latest state, the numbers with their previous values, the rounds
// with their deltas, the open questions, the gaps, what's pinned as won't
// model, the loops' table rows, the watcher's state and the log. Pure, so
// the tests fold made-up events (docs/capture-live.md,
// docs/formats/capture-events.md).

import type { CaptureEvent } from "./api";

export interface StageState {
  status: "running" | "ok" | "fail" | "error" | "cached" | "skipped";
  ms: number;
  round: number;
  summary: string;
  /** For a cached stage: how the replayed run of it ended, and the run it came from. */
  result?: string;
  fromRun?: string;
}

export interface Metric {
  value: number;
  /** The value before this one, from this round or an earlier one. */
  prev: number | null;
  round: number;
  stage?: string;
  /** Over this is a failure (a tracked speed metric). */
  budget?: number;
}

export interface RoundSummary {
  round: number;
  ms: number;
  failed: number;
  deltas: Record<string, { value: number; prev: number | null }>;
  /** Open questions and gaps after the round, less the pinned ones. */
  open: number;
}

/** Something a stage found that a person can pin: a gap, a differing fact, a failing check. */
export interface Found {
  what: string;
  id: string;
  status: string;
  text: string;
  round: number;
  /** What the stage said beside the line: a failing check's values and frame, a gap's frames (docs/formats/capture-events.md). */
  data?: Record<string, unknown>;
}

export interface Question {
  id: string;
  text: string;
  path?: string;
  candidates: { from: string; value: string }[];
  round: number;
  /** Set once an answer to it has been applied. */
  answered?: string;
}

export interface LiveState {
  /** The last event's sequence number; the next poll asks for those after it. */
  seq: number;
  round: number;
  /** Stage names in the order they first appeared. */
  order: string[];
  stages: Record<string, StageState>;
  metrics: Record<string, Metric>;
  rounds: RoundSummary[];
  questions: Record<string, Question>;
  /** The newest events last, at most LOG_SIZE. */
  log: CaptureEvent[];
  /** Recordings the run made or was given, to open in the inspector beside the panel. */
  recordings: { id: string; text: string }[];
  /** The round whose preview build finished last, so the panel reloads it. */
  previewRound?: number;
  running: boolean;
  /** What the run's watcher is doing (`run.state`), when one is. */
  watch?: { state: string; reason: string };
  /** Question IDs, model paths and gap IDs pinned as won't model. */
  pins: string[];
  /** Gaps, differing facts and failing checks, by what then id, the latest of each. */
  found: Record<string, Found>;
  /** Each loop's table (refine, catalog): its rows in order. */
  tables: Record<string, { index: number; round: number; cells: Record<string, unknown> }[]>;
}

export const LOG_SIZE = 300;

export function emptyLive(): LiveState {
  return { seq: 0, round: 0, order: [], stages: {}, metrics: {}, rounds: [], questions: {}, log: [], recordings: [], running: false, pins: [], found: {}, tables: {} };
}

const str = (v: unknown): string => (typeof v === "string" ? v : v === undefined || v === null ? "" : JSON.stringify(v));
const num = (v: unknown): number => (typeof v === "number" ? v : 0);

/** Folds events (in sequence order) into a state; events at or below `s.seq` are ignored. */
export function fold(s: LiveState, events: CaptureEvent[]): LiveState {
  const next: LiveState = {
    ...s,
    order: [...s.order],
    stages: { ...s.stages },
    metrics: { ...s.metrics },
    rounds: [...s.rounds],
    questions: { ...s.questions },
    log: [...s.log],
    recordings: [...s.recordings],
    pins: [...s.pins],
    found: { ...s.found },
    tables: { ...s.tables },
  };
  for (const e of events) {
    if (e.seq <= next.seq) continue;
    next.seq = e.seq;
    next.round = Math.max(next.round, e.round);
    const d = e.data ?? {};
    const stage = e.stage;
    if (stage && !next.order.includes(stage) && (e.kind === "stage.start" || e.kind === "stage.end")) next.order.push(stage);
    switch (e.kind) {
      case "round.start":
        next.running = true;
        break;
      case "stage.start":
        if (stage) next.stages[stage] = { status: "running", ms: 0, round: e.round, summary: "" };
        break;
      case "stage.end":
        if (stage) {
          const status = str(d.status) as StageState["status"];
          next.stages[stage] = { status, ms: num(d.ms), round: e.round, summary: str(d.summary), result: typeof d.result === "string" ? d.result : undefined, fromRun: typeof d.from_run === "string" ? d.from_run : undefined };
          if (stage === "preview" && status === "ok") next.previewRound = e.round;
        }
        break;
      case "item":
        if (d.what === "recording" && typeof d.id === "string" && !next.recordings.some((r) => r.id === d.id)) {
          next.recordings.push({ id: d.id, text: str(d.text) });
        }
        if (FOUND.includes(str(d.what))) {
          next.found[`${str(d.what)}:${str(d.id)}`] = {
            what: str(d.what),
            id: str(d.id),
            status: str(d.status),
            text: str(d.text),
            round: e.round,
            data: d.data && typeof d.data === "object" ? (d.data as Record<string, unknown>) : undefined,
          };
        }
        break;
      case "row": {
        const table = str(d.table);
        next.tables[table] = [...(next.tables[table] ?? []), { index: num(d.index), round: e.round, cells: (d.cells ?? {}) as Record<string, unknown> }];
        break;
      }
      case "run.state":
        next.watch = { state: str(d.state), reason: str(d.reason) };
        break;
      case "metric":
        next.metrics[str(d.key)] = { value: num(d.value), prev: typeof d.prev === "number" ? d.prev : null, round: e.round, stage, budget: typeof d.budget === "number" ? d.budget : undefined };
        break;
      case "question": {
        const id = str(d.id);
        next.questions[id] = {
          id,
          text: str(d.text),
          path: typeof d.path === "string" ? d.path : undefined,
          candidates: Array.isArray(d.candidates) ? (d.candidates as { from: string; value: string }[]) : [],
          round: e.round,
        };
        break;
      }
      case "input": {
        const input = (d.input ?? {}) as { kind?: string; question?: string; path?: string };
        if (input.kind === "answer" && input.question && next.questions[input.question] && d.status === "applied") {
          next.questions[input.question] = { ...next.questions[input.question], answered: str(d.summary) };
        }
        if (input.kind === "pin" && input.path && d.status === "applied" && !next.pins.includes(input.path)) next.pins.push(input.path);
        if (input.kind === "unpin" && input.path && d.status === "applied") next.pins = next.pins.filter((p) => p !== input.path);
        break;
      }
      case "round.end":
        next.running = false;
        next.rounds.push({
          round: e.round,
          ms: num(d.ms),
          failed: num(d.failed),
          deltas: (d.deltas ?? {}) as RoundSummary["deltas"],
          open: num(d.open),
        });
        break;
    }
    if (e.kind !== "metric") next.log.push(e);
  }
  if (next.log.length > LOG_SIZE) next.log = next.log.slice(next.log.length - LOG_SIZE);
  return next;
}

/** The kinds of item a person can pin. */
export const FOUND = ["gap", "fact", "check", "label", "frame"];

/** Whether a pin covers a question, gap or item: by its ID or its path, as the pipeline matches. */
export function isPinned(pins: string[], id: string, path?: string): boolean {
  return pins.some((p) => p === id || (!!path && p === path));
}

/** The questions still waiting for an answer and not pinned, oldest first. */
export function openQuestions(s: LiveState): Question[] {
  return Object.values(s.questions)
    .filter((q) => !q.answered && !isPinned(s.pins, q.id, q.path))
    .sort((a, b) => a.round - b.round || a.id.localeCompare(b.id));
}

/** The questions and items pinned as won't model. */
export function wontModel(s: LiveState): { id: string; pin: string; text: string }[] {
  const out: { id: string; pin: string; text: string }[] = [];
  for (const q of Object.values(s.questions)) {
    const pin = s.pins.find((p) => p === q.id || p === q.path);
    if (pin && !q.answered) out.push({ id: q.id, pin, text: q.text });
  }
  for (const f of Object.values(s.found)) {
    if (s.pins.includes(f.id) && !out.some((o) => o.pin === f.id)) out.push({ id: f.id, pin: f.id, text: `${f.what} ${f.text}` });
  }
  for (const p of s.pins) if (!out.some((o) => o.pin === p)) out.push({ id: p, pin: p, text: "" });
  return out;
}

/** The gaps the last round that ran the gaps stage found, less the pinned ones. */
export function openGaps(s: LiveState): Found[] {
  const gaps = Object.values(s.found).filter((f) => f.what === "gap");
  const last = Math.max(0, ...gaps.map((g) => g.round));
  return gaps.filter((g) => g.round === last && !s.pins.includes(g.id)).sort((a, b) => a.id.localeCompare(b.id));
}

/** The numbers the strip shows first: the scorecard's headline columns. */
export const HEADLINE = [
  "assets.count",
  "facts.total",
  "facts.derived",
  "facts.derived_differ",
  "facts.agree",
  "facts.compared",
  "questions.open",
  "gaps.open",
  "fidelity.interp.passed",
  "fidelity.interp.failed",
  "fidelity.track.recall",
  "mechanics.debt",
  "speed.rom_to_preview_s",
];

/** A number for the strip: whole numbers as they are, shares to three places. */
export function fmt(v: number | null | undefined): string {
  if (v === null || v === undefined) return "-";
  return Number.isInteger(v) ? String(v) : v.toFixed(3);
}

/** A delta as text: `+2`, `-1`, `new`, or empty when it didn't change. */
export function delta(m: { value: number; prev: number | null }): string {
  if (m.prev === null) return "new";
  const d = m.value - m.prev;
  if (d === 0) return "";
  const t = Number.isInteger(d) ? String(d) : d.toFixed(3);
  return d > 0 ? `+${t}` : t;
}

/** An event as one line of the log. */
export function logLine(e: CaptureEvent): string {
  const d = e.data ?? {};
  switch (e.kind) {
    case "stage.start":
      return `${e.stage} started`;
    case "stage.end":
      return `${e.stage} ${str(d.status)}${d.result && d.result !== "ok" ? ` (${str(d.result)})` : ""}${d.from_run ? ` from ${str(d.from_run)}` : ""}${num(d.ms) ? ` in ${num(d.ms)} ms` : ""}${d.summary ? `: ${str(d.summary)}` : ""}`;
    case "item":
      return `${str(d.what)} ${str(d.id)} ${str(d.status)}${d.text ? `: ${str(d.text)}` : ""}${d.pinned ? " (won't model)" : ""}`;
    case "question":
      return `question ${str(d.id)}`;
    case "input":
      return `${str(d.kind)} ${str(d.status)}: ${str(d.summary)}`;
    case "round.start":
      return `round ${e.round} started`;
    case "round.end":
      return `round ${e.round} ended: ${str(d.summary)}`;
    case "run.state":
      return `run ${str(d.state)}${d.reason ? `: ${str(d.reason)}` : ""}`;
    case "row":
      return `${str(d.table)} row ${num(d.index)}`;
    case "run.start":
      return `run ${str(d.run)} started on ${str(d.model)}`;
    // A play session's events (docs/show.md).
    case "play.start":
      return `play session ${str(d.run)} started on ${str(d.name)}`;
    case "play.mark":
      return `mark ${str(d.id)} on frames ${str(d.from)} to ${str(d.to)}${d.note ? `: ${str(d.note)}` : ""}`;
    case "play.stop":
      return `stopped after ${str(d.frames)} frames`;
    case "play.recorded":
      return `recorded ${str(d.id).slice(0, 12)} (${str(d.frames)} frames, hashes ${d.digest_matches ? "agree" : "differ"})`;
    case "show.mark":
      return `mark ${str(d.mark)}: ${str(d.summary)}`;
    // Point and tell (docs/point-and-tell.md).
    case "request":
      return `request ${str(d.id)} at frame ${str(d.frame)}: ${str(d.label)}${d.text ? `: ${str(d.text)}` : ""}`;
    case "request.reply":
      return `reply to request ${str(d.request)} (${str(d.status)}): ${str(d.text)}`;
    default:
      return `${e.kind} ${JSON.stringify(d)}`;
  }
}
