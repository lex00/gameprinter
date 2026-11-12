// The game view's "Refining" section in words (docs/inspector.md, "The
// game view"; docs/interpreter.md, "Refining from the lockstep"): the
// refinement loop's state, each round as a sentence a person reads, what
// it's trying now, and a stuck window's question. The words come from the
// model, as lockfold.ts's do; nothing here names a game.

import type { GameWords } from "./api";
import { measureName } from "./lockfold";
import { thing, words } from "./phrase";

export interface RefineWindow {
  /** The model's frames, counted from where both started. */
  from: number;
  to: number;
  /** The original's frames, which the view's slider counts. */
  original_from: number;
  original_to: number;
  seconds_from: number;
  seconds_to: number;
}

export interface RefineRound {
  round: number;
  window: RefineWindow;
  mechanic: string;
  param: string;
  unit: string;
  old: string;
  new: string;
  window_splits: { before: number; after: number };
  exact: boolean;
  first_split: { before?: number | null; after?: number | null };
  next_window?: number | null;
  together: { before: number; after: number };
}

export interface ShapeMeasure {
  path: string;
  mechanic: string;
  original: string[];
  model: string[];
  why: string;
  proposal: string;
}

export interface ShapeQuestion {
  id: string;
  text: string;
  window: RefineWindow;
  mechanic: string;
  measures: ShapeMeasure[];
  proposal: string;
}

export type RefineStatus = "none" | "running" | "stuck" | "done" | "stopped" | "failed";

export interface RefineState {
  recording: string;
  state: RefineStatus;
  running: boolean;
  from?: number | null;
  window?: number;
  current?: { round: number; window: RefineWindow };
  trying?: { round: number; mechanic: string; param: string; unit: string; value: string } | null;
  rounds: RefineRound[];
  /** How many times the preview's bundle was rewritten. */
  bundle?: number;
  /** A loop scoped to one moment: which, and the mechanic named. */
  moment?: { kind: string; n: number } | string | null;
  mechanic?: string | null;
  result?: {
    /** A scoped loop's kept value, measured on every other moment of the kind. */
    also?: AlsoCheck | null;
    scope?: { moment: unknown; mechanic: string | null; until: number | null };
    stop: string;
    questions: ShapeQuestion[];
    first_split: { before?: number | null; after?: number | null };
    together: { before: number; after: number; measured: number };
  };
  error?: string;
}

const PARAM: Record<string, string> = {
  acceleration: "walking speed-up",
  run_acceleration: "running speed-up",
  deceleration: "slow-down",
  skid_deceleration: "slow-down when turning",
  walk_speed: "walking top speed",
  run_speed: "running top speed",
  air_speed: "top speed in the air",
  gravity: "pull down",
  gravity_held: "pull down while jump is held",
  speed: "speed",
  max_fall: "top falling speed",
  release_cap: "speed when jump is let go",
  bounce: "bounce",
  lead: "lead",
  every: "time between changes",
};

const KIND: Record<string, string> = { jump: "jump", run: "run", walk: "walk", patrol: "patrol", fall: "fall" };

/** A number the loop changes, as a person says it: "Mario's running speed-up", "Mario's jump pull down (band 2)". */
export function paramName(g: GameWords | undefined, path: string): string {
  const m = /^entities\.([a-z0-9_]+)\.([a-z_]+)\.(?:([a-z_]+)\[(\d+)\]\.)?([a-z_]+)$/.exec(path);
  if (m) {
    const [, who, comp, , band, field] = m;
    let what = PARAM[field] ?? words(field);
    if (field === "speed" && comp === "jump") what = "jump speed";
    else if (["gravity", "gravity_held", "max_fall", "release_cap"].includes(field) && KIND[comp]) what = `${KIND[comp]} ${what}`;
    return `${thing(g, who)}'s ${what}${band !== undefined ? ` (band ${Number(band) + 1})` : ""}`;
  }
  const parts = path.split(".");
  const last = parts.at(-1) ?? path;
  if (parts[0] === "cameras") return `the view's ${PARAM[last] ?? words(last)}`;
  if (parts[0] === "counters") return `the ${words(parts[1] ?? "")}'s ${PARAM[last] ?? words(last)}`;
  return words(path);
}

/** A unit after a number: an acceleration is "a frame" (more speed each frame). */
export function unitWords(unit: string): string {
  if (unit === "px/frame^2") return "a frame";
  if (unit === "px/frame") return "px a frame";
  return unit;
}

/** A number short enough to read: exact up to four places, else three. */
export function short(v: string): string {
  const n = Number(v);
  if (!Number.isFinite(n)) return v;
  const places = (v.split(".")[1] ?? "").length;
  return places <= 4 ? v : n.toFixed(3);
}

const secs = (s: number) => s.toFixed(1);

/** "Frames 1 to 10 (0.0 to 0.2 s)". */
export function windowWords(w: RefineWindow): string {
  return `Frames ${w.from} to ${w.to} (${secs(w.seconds_from)} to ${secs(w.seconds_to)} s)`;
}

/**
 * One round as a sentence: "Frames 1 to 10 (0.0 to 0.2 s): Mario's running
 * speed-up 0.037 -> 0.0625 a frame. Now exact; next window starts at frame 31."
 */
export function roundLine(g: GameWords | undefined, r: RefineRound): string {
  const unit = unitWords(r.unit);
  const change = `${paramName(g, r.param)} ${short(r.old)} -> ${short(r.new)}${unit ? ` ${unit}` : ""}.`;
  const after = r.exact
    ? r.next_window !== undefined && r.next_window !== null
      ? `Now exact; next window starts at frame ${r.next_window}.`
      : "Now exact, and nothing splits after it."
    : `Closer: ${count(r.window_splits.before)} in the window before, ${r.window_splits.after} now.`;
  return `${windowWords(r.window)}: ${change} ${after}`;
}

const count = (n: number) => (n === 1 ? "1 split" : `${n} splits`);

/** What the loop tries now: "trying: Mario's running speed-up = 0.05". */
export function tryingLine(g: GameWords | undefined, t: NonNullable<RefineState["trying"]>): string {
  return `trying: ${paramName(g, t.param)} = ${short(t.value)}`;
}

/** The loop's state in one line. */
export function refineStatus(s: RefineState): string {
  const w = s.current?.window ?? s.result?.questions[0]?.window;
  const frames = w ? `frames ${w.from} to ${w.to}` : "";
  switch (s.state) {
    case "running":
      return w ? `Refining: ${frames}, round ${s.current?.round ?? 1}` : "Refining: measuring the two games";
    case "stuck":
      return `Stuck at ${frames}: a question for you`;
    case "done": {
      const left = s.result?.first_split.after;
      if (left !== undefined && left !== null) return `Stopped after ${s.rounds.length} rounds: exact to frame ${left - 1}`;
      return `Done: exact to frame ${Math.max(0, (s.result?.together.measured ?? 1) - 1)}`;
    }
    case "stopped":
      return w ? `Stopped at ${frames}` : "Stopped";
    case "failed":
      return `The loop couldn't run: ${s.error ?? "no reason given"}`;
    default:
      return "";
  }
}

/** A stuck window's measure, in words: "Mario's position across: the original 40.188, 40.078; the model 40.063, 40.313." */
export function measureWords(g: GameWords | undefined, m: ShapeMeasure, n = 5): string {
  const vals = (v: string[]) => v.slice(0, n).map(short).join(", ") + (v.length > n ? ", ..." : "");
  const name = measureName(g, m.path);
  return `${name.charAt(0).toUpperCase() + name.slice(1)}: the original ${vals(m.original)}; the model ${vals(m.model)}.`;
}

/** A moment the check measured, before the loop and after. */
export interface AlsoMoment {
  n: number;
  from: number;
  to: number;
  seconds: number;
  original_height: number | null;
  before: { apart: number; found: boolean; offset: number | null; height: number | null; measures: { path: string; max: number }[] };
  after: { apart: number; found: boolean; offset: number | null; height: number | null; measures: { path: string; max: number }[] };
}

/** `also`: the kept value on every other moment of the kind (docs/interpreter.md, "Moments"). */
export interface AlsoCheck {
  kind: string;
  others: number;
  better: number;
  worse: number;
  same: number;
  line: string;
  moment: AlsoMoment | null;
  worse_moments: AlsoMoment[];
}

const KIND_PLURAL: Record<string, [string, string]> = {
  jump: ["jump", "jumps"],
  landing: ["landing", "landings"],
  death: ["death", "deaths"],
  life_lost: ["life lost", "lives lost"],
  counter: ["change", "changes"],
  level: ["level change", "level changes"],
};

/** The check in a sentence: "Also better on 280 of 312 other jumps, worse on 4." */
export function alsoLine(a: AlsoCheck): string {
  const [one, many] = KIND_PLURAL[a.kind] ?? [a.kind, `${a.kind}s`];
  if (!a.others) return `There is no other ${one} in the recording to check it on.`;
  const worse = a.worse ? `, worse on ${a.worse}` : ", worse on none";
  return `Also better on ${a.better} of ${a.others} other ${a.others === 1 ? one : many}${worse}.`;
}

/** A moment the kept value made worse, in words: "Jump 17, 66.9 s: further from the original there (12 measure-frames apart, was 3); the model goes up 40 px, was 60 px". */
export function worseLine(kind: string, m: AlsoMoment): string {
  const name = (KIND_PLURAL[kind]?.[0] ?? kind).replace(/^./, (c) => c.toUpperCase());
  const h = (x: { height: number | null }) => (x.height === null ? "doesn't" : `${Math.round(x.height * 10) / 10} px`);
  const heights = kind === "jump" ? `; the model goes up ${h(m.after)}, was ${h(m.before)}${m.original_height !== null ? `, the original ${Math.round(m.original_height * 10) / 10} px` : ""}` : "";
  return `${name} ${m.n}, ${m.seconds.toFixed(1)} s: further from the original there (${m.after.apart} measure-frames apart, was ${m.before.apart})${heights}`;
}

/**
 * Where "refine this" on a moment is, for the view and a guide that drives
 * it: idle (not started), refining (round k), done (with the check on the
 * other moments), stuck (with the question), stopped or failed. `says` is
 * what is happening, `next` what comes after.
 */
export interface MomentRefinePhase {
  state: "idle" | "refining" | "done" | "stuck" | "stopped" | "failed";
  says: string;
  next: string;
}

export function momentRefinePhase(st: RefineState | undefined, name: string, mechanic: string | null): MomentRefinePhase {
  const what = mechanic ? `${words(mechanic.split(".").pop() ?? mechanic)}'s numbers` : "the numbers of its mechanic";
  if (!st || st.state === "none" || !st.moment) return { state: "idle", says: `${name}: refine tries ${what} in its frames only.`, next: `Press "refine this" to start.` };
  switch (st.state) {
    case "running":
      return { state: "refining", says: `Refining ${name.toLowerCase()}: round ${st.current?.round ?? 1}${st.trying ? `, trying ${st.trying.param.split(".").pop()} = ${st.trying.value}` : ""}.`, next: "Each kept number is then checked on every other one of its kind." };
    case "done": {
      const a = st.result?.also;
      const kept = st.rounds.length ? `${st.rounds.length === 1 ? "1 number" : `${st.rounds.length} numbers`} changed` : "nothing needed changing";
      return { state: "done", says: `Done with ${name.toLowerCase()}: ${kept}.${a ? ` ${alsoLine(a)}` : ""}`, next: a?.worse ? "The ones it made worse are questions below." : "Pick another moment, or play this one again." };
    }
    case "stuck":
      return { state: "stuck", says: `Stuck on ${name.toLowerCase()}: no numbers of ${what} make it match.`, next: "Answer the question below." };
    case "stopped":
      return { state: "stopped", says: `Stopped refining ${name.toLowerCase()}.`, next: `Press "refine this" to go on.` };
    case "failed":
      return { state: "failed", says: `Refining ${name.toLowerCase()} couldn't run: ${st.error ?? "no reason given"}.`, next: "" };
    default:
      return { state: "idle", says: "", next: "" };
  }
}
